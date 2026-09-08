//! Operation 层基础行为：System 即可承载响应、深度子操作链、
//! 生命提交的历史值与反应、参数修改窗口、攻击资格复查与流程检查点序列。

use std::cell::RefCell;
use std::rc::Rc;

use duel::core::{
    business::damage_reduction::{add_damage_reduction, register_damage_reduction_rule},
    event::{Checkpoint, Event, EventType},
    install_default_rules,
    operation::{ExecutionContext, HpChange, Operation, OperationError, OperationResult},
    player::Player,
    system::{Fact, NoticeKind, Priority, Subject, System},
    AttackOperation, Damage, DuelRunner, PlayerId, World,
};

/// 受伤后回复 3 点的响应 System，用于检验提交返回前反应是否完成。
#[derive(Debug)]
struct HealAfterDamage(PlayerId);

impl System for HealAfterDamage {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::HpChanged), Priority::Default)]
    }

    fn candidates(&self, _fact: &Fact<'_>, _query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        vec![Subject::Standalone]
    }

    fn respond(
        &self,
        fact: &Fact<'_>,
        _subject: Subject,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        let Some(Event::HpChanged {
            target_id,
            old_hp,
            new_hp,
        }) = fact.event()
        else {
            return Ok(vec![]);
        };
        if *target_id == self.0 && new_hp < old_hp {
            Ok(vec![Box::new(duel::core::Heal::new(self.0, 3))])
        } else {
            Ok(vec![])
        }
    }
}

#[derive(Debug)]
struct DamageOnce(usize);

impl Operation for DamageOnce {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<(OperationResult, Option<Box<dyn std::any::Any>>), OperationError> {
        let change = context
            .modify_hp(self.0, -3)
            .expect("提交应成功")
            .expect("玩家应存在");
        Ok((OperationResult::Completed, Some(Box::new(change))))
    }
}

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

#[test]
fn hp_submission_returns_history_and_runs_reactions() {
    let mut world = World::new();
    let player = world.add_player(Player::new("A".into(), 10, 1));
    world.add_system(HealAfterDamage(player));
    let outcome = world.execute(DamageOnce(player)).expect("操作应成功");
    let value = outcome.1.expect("应返回生命结果");
    let change = value
        .downcast_ref::<HpChange>()
        .expect("类型应匹配")
        .to_owned();
    // 返回的是本次提交的历史事实（10 → 7）……
    assert_eq!((change.old_hp, change.new_hp), (10, 7));
    // ……而当前状态已包含反应（回复 3 点）。
    assert_eq!(world.get_player(player).expect("玩家应存在").hp(), 10);
}

#[test]
fn damage_operation_applies_parameter_modifiers_before_submission() {
    let mut world = World::new();
    let source = world.add_player(Player::new("A".into(), 10, 1));
    let target = world.add_player(Player::new("B".into(), 20, 1));
    register_damage_reduction_rule(&mut world);
    add_damage_reduction(&mut world, target, target, 0.25);
    let outcome = world
        .execute(Damage::new(Some(source), target, 8))
        .expect("伤害操作应成功");
    let value = outcome.1.expect("应返回生命结果");
    let change = value.downcast_ref::<HpChange>().expect("类型应匹配");
    assert_eq!((change.old_hp, change.new_hp), (20, 14));
}

/// 陷阱：攻击开始事件对来源反噬 10 点伤害。
#[derive(Debug)]
struct Trap(PlayerId);

impl System for Trap {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(
            NoticeKind::Event(EventType::BeforePlayerAttack),
            Priority::Default,
        )]
    }

    fn candidates(&self, _fact: &Fact<'_>, _query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        vec![Subject::Standalone]
    }

    fn respond(
        &self,
        fact: &Fact<'_>,
        _subject: Subject,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        let Some(Event::BeforePlayerAttack { source_id, .. }) = fact.event() else {
            return Ok(vec![]);
        };
        if *source_id != self.0 {
            return Ok(vec![]);
        }
        Ok(vec![Box::new(Damage::new(None, self.0, 10))])
    }
}

#[test]
fn attack_operation_rechecks_source_after_before_attack_reactions() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let source = world.add_player(Player::new("A".into(), 10, 4));
    let target = world.add_player(Player::new("B".into(), 10, 1));
    world.add_system(Trap(source));
    let result = world.execute(AttackOperation::new(source));
    assert!(result.is_ok());
    // 陷阱先杀死来源；攻击复查后放弃，不得对目标盲出伤害。
    assert!(world.get_player(source).is_none(), "来源应死于陷阱");
    assert_eq!(
        world.get_player(target).expect("目标应存在").hp(),
        10,
        "攻击在反应后复查来源，未继续提交伤害"
    );
}

/// 记录检查点阶段的探针。
#[derive(Debug)]
struct CheckpointRecorder(Rc<RefCell<Vec<Checkpoint>>>);

impl System for CheckpointRecorder {
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
        fact: &Fact<'_>,
        _subject: Subject,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        if let Some(Event::Checkpoint { phase, .. }) = fact.event() {
            self.0.borrow_mut().push(*phase);
        }
        Ok(vec![])
    }
}

#[test]
fn duel_operation_owns_round_turn_and_checkpoint_progression() {
    let mut world = World::new();
    world.add_player(Player::new("A".into(), 10, 0));
    world.add_player(Player::new("B".into(), 10, 0));
    let phases = Rc::new(RefCell::new(Vec::new()));
    world.add_system(CheckpointRecorder(phases.clone()));
    world.run_with_max_rounds(1).expect("对局应正常结束");
    assert!(world.is_end());
    // 两个玩家都没有行动：依次经过 DuelStart、RoundStart、
    // 每名玩家的 TurnStart/TurnEnd，最后 RoundEnd。
    assert_eq!(
        *phases.borrow(),
        vec![
            Checkpoint::DuelStart,
            Checkpoint::RoundStart,
            Checkpoint::TurnStart,
            Checkpoint::TurnEnd,
            Checkpoint::TurnStart,
            Checkpoint::TurnEnd,
            Checkpoint::RoundEnd,
        ]
    );
    assert!(world.get_player(0).is_some());
    assert!(world.get_player(1).is_some());
}

#[test]
fn duel_operation_stops_at_its_own_round_limit() {
    let mut world = World::new();
    world.add_player(Player::new("A".into(), 10, 0));
    world.add_player(Player::new("B".into(), 10, 0));
    world.run_with_max_rounds(1).expect("对局应正常结束");
    assert!(world.is_end());
    assert!(world.get_player(0).is_some());
    assert!(world.get_player(1).is_some());
}
