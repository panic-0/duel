//! 第二轮审查回归测试：对应审查意见 F1–F7 的修复行为。
//! 攻击失效用例采用已确认的默认普攻规则（来源在伤害提交前失效则停止）；
//! 旧配置兼容用例已随旧执行面整体删除而移除。

use std::{cell::Cell, rc::Rc};

use duel::core::{
    business::{
        revival::{add_revival, register_revival_system},
        skills::{Abilities, Ability},
    },
    event::{Checkpoint, Event, EventType},
    install_default_rules,
    operation::{
        completed, ExecutionContext, Operation, OperationError, OperationOutcome,
        RemovePlayerOperation,
    },
    player::Player,
    state::GameState,
    system::{Fact, NoticeKind, Priority, Subject, System},
    Attack, AttackOperation, Damage, DeathOperation, Heal, OperationResult, PlayerId,
    RoundOperation, TurnOperation, World,
};

// —— F1：稳定技能槽位，避免过滤列表索引漂移 ——

/// 未满血时治疗的技能；治疗完成后即不可用。
#[derive(Debug)]
struct HealIfWounded;
impl Ability for HealIfWounded {
    fn operation(&self, source: PlayerId, state: &GameState) -> Option<Box<dyn Operation>> {
        let player = state.get_player(source)?;
        if player.hp() < player.max_hp() {
            Some(Box::new(Heal::new(source, player.max_hp() - player.hp())))
        } else {
            None
        }
    }
}

#[test]
fn normal_action_cursor_does_not_skip_attack_after_conditional_heal_disappears() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 3));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    world.get_player_mut(a).unwrap().set_hp(9); // 仅用于测试初始化。
    world.add_data(
        Some(a),
        Abilities::new(a, vec![Box::new(HealIfWounded), Box::new(Attack)]),
    );

    world
        .execute(TurnOperation {
            round: 1,
            player_id: a,
        })
        .expect("回合应正常执行");

    assert_eq!(world.get_player(a).unwrap().hp(), 10);
    assert_eq!(
        world.get_player(b).unwrap().hp(),
        17,
        "治疗后不可用的技能不应让普攻因过滤列表索引左移而被跳过"
    );
}

// —— F2：失败后不再执行后续变化 ——

#[derive(Debug)]
struct Fail;
impl Operation for Fail {
    fn execute(
        self: Box<Self>,
        _: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        Err(OperationError::Failed("回归测试的故意失败".into()))
    }
}

/// 对指定事件一律失败响应的 System。
#[derive(Debug)]
struct FailureOn(EventType);

impl System for FailureOn {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(self.0), Priority::Default)]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        fact.event()
            .map(|_| vec![Subject::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(vec![Box::new(Fail)])
    }
}

/// 故意忽略提交结果：即使调用方不传播，失败也必须阻止后续提交。
#[derive(Debug)]
struct ContinueAfterFailedHpChange {
    a: PlayerId,
    b: PlayerId,
}
impl Operation for ContinueAfterFailedHpChange {
    fn execute(
        self: Box<Self>,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        let _ = ctx.modify_hp(self.a, -1);
        let _ = ctx.modify_hp(self.b, -1);
        completed()
    }
}

#[test]
fn failed_hp_reaction_must_not_allow_later_world_mutation() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 10, 0));
    world.add_system(FailureOn(EventType::HpChanged));

    let result = world.execute(ContinueAfterFailedHpChange { a, b });

    assert!(result.is_err());
    assert!(world.is_operation_failed());
    assert_eq!(
        world.get_player(a).unwrap().hp(),
        9,
        "不要求回滚第一次已提交的变化"
    );
    assert_eq!(
        world.get_player(b).unwrap().hp(),
        10,
        "错误发生后不应继续提交第二次变化"
    );
}

#[derive(Debug)]
struct Count(Rc<Cell<usize>>);
impl Operation for Count {
    fn execute(
        self: Box<Self>,
        _: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        self.0.set(self.0.get() + 1);
        completed()
    }
}

#[derive(Debug)]
struct ContinueChildAfterPublishError(Rc<Cell<usize>>);
impl Operation for ContinueChildAfterPublishError {
    fn execute(
        self: Box<Self>,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        // 故意不传播发布错误：即使调用方忽略，失败也必须阻止后续子操作。
        let _ = ctx.publish(Event::RoundStart { round: 1 });
        ctx.execute(Count(self.0.clone()))?;
        completed()
    }
}

#[test]
fn a_child_must_not_start_when_the_world_already_records_failure() {
    let mut world = World::new();
    let count = Rc::new(Cell::new(0));
    world.add_system(FailureOn(EventType::RoundStart));

    let result = world.execute(ContinueChildAfterPublishError(count.clone()));

    assert!(result.is_err());
    assert_eq!(count.get(), 0, "不能先执行新子操作，再检查先前的失败标记");
}

// —— F3：攻击的资格复查与结果通知 ——

/// PlayerAttack 响应中杀死攻击者。
#[derive(Debug)]
struct KillSourceAtPlayerAttack;
impl System for KillSourceAtPlayerAttack {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(
            NoticeKind::Event(EventType::PlayerAttack),
            Priority::Default,
        )]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        matches!(fact.event(), Some(Event::PlayerAttack { .. }))
            .then(|| vec![Subject::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        fact: &Fact<'_>,
        _subject: Subject,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        let Some(Event::PlayerAttack { source_id, .. }) = fact.event() else {
            return Ok(vec![]);
        };
        Ok(vec![Box::new(Damage::new(None, *source_id, 10))])
    }
}

#[test]
fn attack_revalidates_after_the_last_pre_submission_event() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 3));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    world.add_system(KillSourceAtPlayerAttack);

    world
        .execute(AttackOperation::new(a))
        .expect("攻击应正常结算");

    assert!(world.get_player(a).is_none());
    assert_eq!(
        world.get_player(b).unwrap().hp(),
        20,
        "采用默认普攻规则：来源在伤害提交前失效则不再扣血"
    );
}

/// PlayerAttack 响应中直接移除目标。
#[derive(Debug)]
struct RemoveTargetAtPlayerAttack;
impl System for RemoveTargetAtPlayerAttack {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(
            NoticeKind::Event(EventType::PlayerAttack),
            Priority::Default,
        )]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        matches!(fact.event(), Some(Event::PlayerAttack { .. }))
            .then(|| vec![Subject::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        fact: &Fact<'_>,
        _subject: Subject,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        let Some(Event::PlayerAttack { target_id, .. }) = fact.event() else {
            return Ok(vec![]);
        };
        Ok(vec![Box::new(RemovePlayerOperation(*target_id))])
    }
}

#[derive(Debug)]
struct ObserveAfterAttackDamage(Rc<Cell<u64>>);
impl System for ObserveAfterAttackDamage {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(
            NoticeKind::Event(EventType::AfterPlayerAttack),
            Priority::Default,
        )]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        matches!(fact.event(), Some(Event::AfterPlayerAttack { .. }))
            .then(|| vec![Subject::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        fact: &Fact<'_>,
        _subject: Subject,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        if let Some(Event::AfterPlayerAttack { damage, .. }) = fact.event() {
            self.0.set(*damage);
        }
        Ok(vec![])
    }
}

#[test]
fn skipped_damage_is_not_reported_as_a_positive_submitted_damage() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 3));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    let reported = Rc::new(Cell::new(0));
    world.add_system(RemoveTargetAtPlayerAttack);
    world.add_system(ObserveAfterAttackDamage(reported.clone()));

    let (result, _) = world
        .execute(AttackOperation::new(a))
        .expect("目标失效应按跳过处理");

    assert!(world.get_player(b).is_none());
    assert_eq!(result, OperationResult::Skipped);
    assert_eq!(
        reported.get(),
        0,
        "Damage 没有提交时不得伪造正伤害通知，也不能回退到原始伤害"
    );
}

// —— F5a：同一次死亡不重复进入死亡前流程 ——

/// 统计死亡前通知次数的探针。
#[derive(Debug)]
struct BeforeDeathCounter {
    priority: Priority,
    count: Rc<Cell<usize>>,
}
impl System for BeforeDeathCounter {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(
            NoticeKind::Event(EventType::BeforePlayerDeath),
            self.priority,
        )]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        matches!(fact.event(), Some(Event::BeforePlayerDeath(_)))
            .then(|| vec![Subject::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(vec![Box::new(Count(self.count.clone()))])
    }
}

/// 在死亡前通知里对同一玩家再次请求死亡的 System（每次都请求，验证防重入）。
#[derive(Debug)]
struct RepeatSameDeath {
    player: PlayerId,
}
impl System for RepeatSameDeath {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(
            NoticeKind::Event(EventType::BeforePlayerDeath),
            Priority::Default,
        )]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        matches!(fact.event(), Some(Event::BeforePlayerDeath(_)))
            .then(|| vec![Subject::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(vec![Box::new(DeathOperation {
            player_id: self.player,
        })])
    }
}

#[test]
fn reentering_the_same_pending_death_does_not_repeat_before_death_effects() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let count = Rc::new(Cell::new(0));
    world.add_system(BeforeDeathCounter {
        priority: Priority::Modify,
        count: count.clone(),
    });
    world.add_system(RepeatSameDeath { player: a });

    world
        .execute(Damage::new(None, a, 10))
        .expect("有限的死亡链");

    assert!(world.get_player(a).is_none());
    assert_eq!(
        count.get(),
        1,
        "同一次正在进行的死亡不能重复执行死亡前副作用"
    );
}

// —— F5b：救回后分发器不再压制剩余死亡前监听者 ——

#[test]
fn the_dispatcher_does_not_suppress_remaining_before_death_hooks_after_rescue() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let count = Rc::new(Cell::new(0));
    register_revival_system(&mut world);
    add_revival(&mut world, a);
    world.add_system(BeforeDeathCounter {
        priority: Priority::Final,
        count: count.clone(),
    });

    world.execute(Damage::new(None, a, 10)).expect("救回");

    assert_eq!(world.get_player(a).unwrap().hp(), 5);
    assert_eq!(
        count.get(),
        1,
        "候选仍存在；应由 System 自己判断新状态，而非 World 特判跳过整段死亡前通知"
    );
}

// —— F7：独立 Round 拥有自己的轮末处理 ——

#[derive(Debug)]
struct RoundEndCounter(Rc<Cell<usize>>);
impl System for RoundEndCounter {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![
            (NoticeKind::Event(EventType::RoundEnd), Priority::Default),
            (NoticeKind::Event(EventType::Checkpoint), Priority::Default),
        ]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        match fact.event() {
            Some(Event::RoundEnd { .. }) => vec![Subject::Standalone],
            Some(Event::Checkpoint { phase, .. }) if *phase == Checkpoint::RoundEnd => {
                vec![Subject::Standalone]
            }
            _ => vec![],
        }
    }
    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(vec![Box::new(Count(self.0.clone()))])
    }
}

#[test]
fn a_standalone_round_finishes_its_own_end_event_and_checkpoint() {
    let mut world = World::new();
    world.add_player(Player::new("A".into(), 10, 0));
    world.add_player(Player::new("B".into(), 10, 0));
    let count = Rc::new(Cell::new(0));
    world.add_system(RoundEndCounter(count.clone()));

    world
        .execute(RoundOperation { round: 1 })
        .expect("一个完整回合");

    assert_eq!(
        count.get(),
        2,
        "RoundOperation 应拥有轮末事件及检查点，而不要求调用者复制 DuelOperation 的收尾代码"
    );
}
