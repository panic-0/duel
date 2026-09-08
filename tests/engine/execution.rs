//! 子操作调度、反应深度与已产生操作的存活规则。

use crate::support::op_system;
use duel::core::{
    event::{Event, EventType},
    install_default_rules,
    operation::{
        completed, EmitEvent, ExecutionContext, Operation, OperationError, OperationOutcome,
        OperationResult,
    },
    player::Player,
    query::Query,
    system::{Fact, NoticeKind, Priority, Subject, System},
    BuffId, Damage, DuelRunner, Heal, PlayerId, World,
};
use std::{cell::RefCell, rc::Rc};

/// 只在检查点上提出治疗的响应 System。
#[derive(Debug)]
struct CheckpointHeal {
    player_id: usize,
}

impl System for CheckpointHeal {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::Checkpoint), Priority::Default)]
    }

    fn candidates(&self, fact: &Fact<'_>, _query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        matches!(fact.event(), Some(Event::Checkpoint { .. }))
            .then(|| vec![Subject::Standalone])
            .unwrap_or_default()
    }

    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(vec![Box::new(duel::core::Heal::new(self.player_id, 1))])
    }
}

#[test]
fn system_can_be_implemented_with_operations_only() {
    let mut world = World::new();
    let player = world.add_player(Player::new("A".into(), 10, 0));
    world.add_system(CheckpointHeal { player_id: player });
    world.run_with_max_rounds(0).expect("对局应正常结束");
    assert!(world.is_end());
}

#[derive(Debug)]
struct Chain {
    remaining: usize,
    count: Rc<std::cell::Cell<usize>>,
}

impl Operation for Chain {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<(OperationResult, Option<Box<dyn std::any::Any>>), OperationError> {
        self.count.set(self.count.get() + 1);
        if self.remaining > 0 {
            context.spawn(Self {
                remaining: self.remaining - 1,
                count: self.count.clone(),
            });
        }
        Ok((OperationResult::Completed, None))
    }
}

#[test]
fn operation_stack_handles_deep_child_chains() {
    let mut world = World::new();
    let count = Rc::new(std::cell::Cell::new(0));
    let result = world.execute(Chain {
        remaining: 20_000,
        count: count.clone(),
    });
    assert!(result.is_ok());
    assert_eq!(count.get(), 20_001);
}

#[derive(Debug)]
struct CounterMark;

#[derive(Debug)]
struct CounterAndHealSystem {
    owner: PlayerId,
    ally: PlayerId,
}

impl System for CounterAndHealSystem {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::RoundStart), Priority::Default)]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &Query<'_>) -> Vec<Subject> {
        matches!(fact.event(), Some(Event::RoundStart { .. }))
            .then(|| vec![Subject::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        _query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(vec![
            Box::new(Damage::new(None, self.owner, 10)),
            Box::new(Heal::new(self.ally, 3)),
        ])
    }
}

#[test]
fn system_returned_operation_list_survives_source_destruction() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let owner = world.add_player(Player::new("Owner".into(), 10, 0));
    let ally = world.add_player(Player::new("Ally".into(), 20, 0));
    world.get_player_mut(ally).unwrap().set_hp(15);
    let mark = world.add_data(Some(owner), CounterMark);
    world.add_system(CounterAndHealSystem { owner, ally });

    world
        .execute(EmitEvent(Event::RoundStart { round: 1 }))
        .expect("事件应正常分发");

    // 反击先使 owner 死亡并销毁其数据；同一响应已返回的治疗仍按序执行。
    assert!(world.get_player(owner).is_none(), "反击应使 owner 死亡");
    assert!(
        world.get_data::<CounterMark>(mark).is_none(),
        "owner 数据应随死亡销毁"
    );
    assert_eq!(
        world.get_player(ally).map(|p| p.hp()),
        Some(18),
        "同一响应已返回的操作列表不得因来源销毁被丢弃"
    );
}

/// 自我延续的事件反应链：每次发布都催生下一次发布。
#[derive(Debug)]
struct PublishChain(u32);

impl Operation for PublishChain {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.try_publish(Event::RoundEnd { round: self.0 + 1 })?;
        completed()
    }
}

#[derive(Debug)]
struct ChainDriver;

impl System for ChainDriver {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::RoundEnd), Priority::Default)]
    }

    fn candidates(&self, fact: &Fact<'_>, _query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        matches!(fact.event(), Some(Event::RoundEnd { .. }))
            .then(|| vec![Subject::Standalone])
            .unwrap_or_default()
    }

    fn respond(
        &self,
        fact: &Fact<'_>,
        _subject: Subject,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        let Some(Event::RoundEnd { round }) = fact.event() else {
            return Ok(vec![]);
        };
        Ok(vec![Box::new(PublishChain(*round))])
    }
}

#[test]
fn audit_runaway_reaction_chain_is_bounded_instead_of_overflowing() {
    let mut world = World::new();
    world.add_system(ChainDriver);
    let result = world.execute(PublishChain(0));
    assert!(result.is_err(), "失控反应链应报错终止，而不是栈溢出");
    assert!(world.is_operation_failed(), "深度上限错误应记录到世界");
}

/// 来源数据：治疗操作显式检查它是否仍存在（是否存在的判断即全部语义）。
#[derive(Debug)]
struct SourceMark;

/// 反击组合：反噬来源致死，随后提出一个无条件治疗和一个显式检查来源的治疗。
#[derive(Debug)]
struct CounterThenHeal {
    owner: PlayerId,
    ally_unconditional: PlayerId,
    ally_conditional: PlayerId,
    source_buff: BuffId,
}

#[derive(Debug)]
struct UnconditionalHeal(PlayerId);

impl Operation for UnconditionalHeal {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.submit(duel::core::ChangeSet::new().heal(self.0, 3))?;
        completed()
    }
}

#[derive(Debug)]
struct HealOnlyIfSourceExists(PlayerId, BuffId);

impl Operation for HealOnlyIfSourceExists {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        if context.data::<SourceMark>(self.1).is_none() {
            return Ok((duel::core::OperationResult::Skipped, None));
        }
        context.submit(duel::core::ChangeSet::new().heal(self.0, 3))?;
        completed()
    }
}

impl Operation for CounterThenHeal {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.execute(Damage::new(None, self.owner, 10))?;
        context.execute(UnconditionalHeal(self.ally_unconditional))?;
        context.execute(HealOnlyIfSourceExists(
            self.ally_conditional,
            self.source_buff,
        ))?;
        completed()
    }
}

#[test]
fn operation_combination_does_not_depend_on_source_survival() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let owner = world.add_player(Player::new("Owner".into(), 10, 0));
    let ally1 = world.add_player(Player::new("Ally1".into(), 20, 0));
    let ally2 = world.add_player(Player::new("Ally2".into(), 20, 0));
    // 仅用于初始化：让治疗量可见。
    world.get_player_mut(ally1).unwrap().set_hp(15);
    world.get_player_mut(ally2).unwrap().set_hp(15);
    let source_buff = world.add_data(Some(owner), SourceMark);

    world
        .execute(CounterThenHeal {
            owner,
            ally_unconditional: ally1,
            ally_conditional: ally2,
            source_buff,
        })
        .expect("组合操作应正常结算");

    assert!(world.get_player(owner).is_none(), "反噬应使来源 owner 死亡");
    assert!(
        world.get_data::<SourceMark>(source_buff).is_none(),
        "来源数据应随 owner 死亡销毁"
    );
    assert_eq!(
        world.get_player(ally1).map(|p| p.hp()),
        Some(18),
        "已产生的无条件治疗不得因来源销毁被自动取消"
    );
    assert_eq!(
        world.get_player(ally2).map(|p| p.hp()),
        Some(15),
        "显式要求来源存在的治疗版本应自行跳过"
    );
}

#[test]
fn long_event_reaction_chain_settles_within_depth_budget() {
    let mut world = World::new();
    let count = Rc::new(RefCell::new(0));
    let sink = count.clone();
    op_system(&mut world, &[EventType::RoundEnd], move |fact, _| {
        if let Some(Event::RoundEnd { round }) = fact.event() {
            *sink.borrow_mut() += 1;
            if *round < 150 {
                return vec![Box::new(EmitEvent(Event::RoundEnd { round: *round + 1 }))];
            }
        }
        vec![]
    });
    world
        .execute(EmitEvent(Event::RoundEnd { round: 1 }))
        .expect("链长在深度预算内应正常结算");
    assert_eq!(*count.borrow(), 150);
}
