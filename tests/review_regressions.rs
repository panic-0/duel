//! 第一轮审查回归测试：断言定稿协议（N1A/C1A/C3A/C4A、错误传播、数值方向）
//! 以及设计审计表的关键场景。owner 过滤断言已按新语义撤回，
//! 跨角色生命周期用例见 review_v3_regressions。

use std::{cell::Cell, rc::Rc};

use duel::core::{
    buff_data::DestructionReason,
    business::{
        damage::{register_damage_rule, Damage, DamageContext, DamageRule},
        damage_reduction::{add_damage_reduction, register_damage_reduction_rule},
        revival::{add_revival, register_revival_system, RevivalData},
        skills::{Abilities, Ability},
    },
    event::{Checkpoint, Event, EventType},
    install_default_rules,
    operation::{
        completed, completed_with, EmitEvent, ExecutionContext, HpChange, Operation,
        OperationError, OperationOutcome, RemoveDataOperation,
    },
    player::Player,
    state::GameState,
    system::{Fact, NoticeKind, Priority, Subject, System},
    AttackOperation, BuffId, DeathOperation, DuelRunner, Heal, PlayerId, World,
};

fn checkpoint() -> Event {
    Event::Checkpoint {
        phase: Checkpoint::ActionEnd,
        round: Some(1),
    }
}

#[derive(Debug)]
struct CountOperation(Rc<Cell<usize>>);
impl Operation for CountOperation {
    fn execute(
        self: Box<Self>,
        _: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        self.0.set(self.0.get() + 1);
        completed()
    }
}

#[derive(Debug)]
struct CheckpointCounter(Rc<Cell<usize>>);
impl System for CheckpointCounter {
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
        Ok(vec![Box::new(CountOperation(self.0.clone()))])
    }
}

#[derive(Debug)]
struct PublishThenRead(Rc<Cell<usize>>);
impl Operation for PublishThenRead {
    fn execute(
        self: Box<Self>,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        ctx.publish(checkpoint())?;
        completed_with(self.0.get())
    }
}

#[test]
fn n1a_publish_finishes_reactions_before_returning_to_operation() {
    let mut world = World::new();
    let count = Rc::new(Cell::new(0));
    world.add_system(CheckpointCounter(count.clone()));
    let (_, value) = world.execute(PublishThenRead(count)).expect("根操作");
    let observed = *value
        .expect("应读取到计数")
        .downcast::<usize>()
        .expect("usize");
    assert_eq!(observed, 1, "publish 在反应运行之前就返回了");
}

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
            Ok(vec![Box::new(Heal::new(self.0, 3))])
        } else {
            Ok(vec![])
        }
    }
}

#[derive(Debug)]
struct DamageThenRead(PlayerId);
impl Operation for DamageThenRead {
    fn execute(
        self: Box<Self>,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        let change = ctx
            .modify_hp(self.0, -4)
            .expect("提交应成功")
            .expect("目标应存在");
        let now = ctx.state().get_player(self.0).expect("目标存活").hp();
        completed_with((change.new_hp, now))
    }
}

#[test]
fn c1a_hp_submission_returns_history_after_finishing_reactions() {
    let mut world = World::new();
    let player = world.add_player(Player::new("A".into(), 10, 0));
    world.add_system(HealAfterDamage(player));
    let (_, value) = world.execute(DamageThenRead(player)).expect("根操作");
    let observed = *value
        .expect("应读取到生命值")
        .downcast::<(u64, u64)>()
        .expect("元组");
    assert_eq!(observed, (6, 9), "历史值应为 6，当前值应包含治疗反应");
}

/// 空数据实例标记，用于验证 C3A 的新增实例语义。
#[derive(Debug)]
struct MarkerData;

/// 为每个 Marker 实例产生候选并计数的 System。
#[derive(Debug)]
struct MarkerCounter(Rc<Cell<usize>>);
impl System for MarkerCounter {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::Checkpoint), Priority::Default)]
    }
    fn candidates(&self, _fact: &Fact<'_>, query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        query
            .instances::<MarkerData>()
            .iter()
            .map(|(id, _, _)| Subject::Instance(*id))
            .collect()
    }
    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        self.0.set(self.0.get() + 1);
        Ok(vec![])
    }
}

#[derive(Debug)]
struct PublishThenAdd;
impl Operation for PublishThenAdd {
    fn execute(
        self: Box<Self>,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        ctx.publish(checkpoint())?;
        ctx.add_data(None, MarkerData)?;
        completed()
    }
}

#[test]
fn n1a_and_c3a_later_added_instance_does_not_receive_previously_published_event() {
    let mut world = World::new();
    let count = Rc::new(Cell::new(0));
    world.add_system(MarkerCounter(count.clone()));
    world.execute(PublishThenAdd).expect("根操作");
    assert_eq!(count.get(), 0, "新增数据实例收到了在它创建之前发布的通知");
}

#[derive(Debug)]
struct FailOperation;
impl Operation for FailOperation {
    fn execute(
        self: Box<Self>,
        _: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        Err(OperationError::Failed("回归测试的故意失败".into()))
    }
}

#[derive(Debug)]
struct FailingSystem;
impl System for FailingSystem {
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
        Ok(vec![Box::new(FailOperation)])
    }
}

#[test]
fn inline_operation_error_reaches_root_and_stops_later_listeners() {
    let mut world = World::new();
    let count = Rc::new(Cell::new(0));
    world.add_system(FailingSystem);
    world.add_system(CheckpointCounter(count.clone()));
    let result = world.execute(EmitEvent(checkpoint()));
    assert!(result.is_err(), "嵌套 Err 被静默忽略");
    assert_eq!(count.get(), 0, "操作失败后更晚的监听者仍然运行了");
}

/// 自定义死亡判断：观察到零血角色的生命变化后提出 Death。
#[derive(Debug)]
struct GlobalDeathRule;
impl System for GlobalDeathRule {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::HpChanged), Priority::Default)]
    }
    fn candidates(&self, fact: &Fact<'_>, query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        if !matches!(fact.event(), Some(Event::HpChanged { .. })) {
            return Vec::new();
        }
        if query.state().get_players().values().any(|p| p.hp() == 0) {
            vec![Subject::Standalone]
        } else {
            Vec::new()
        }
    }
    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(query
            .state()
            .get_players()
            .iter()
            .filter(|(_, p)| p.hp() == 0)
            .map(|(&id, _)| Box::new(DeathOperation { player_id: id }) as Box<dyn Operation>)
            .collect())
    }
}

#[test]
fn ordinary_death_operation_preserves_before_death_rescue_when_triggered_by_global_rule() {
    let mut world = World::new();
    let player = world.add_player(Player::new("A".into(), 10, 0));
    world.add_system(GlobalDeathRule);
    register_revival_system(&mut world);
    add_revival(&mut world, player);
    world
        .execute(Damage::new(None, player, 10))
        .expect("致命伤害");
    assert_eq!(
        world.get_player(player).map(|p| p.hp()),
        Some(5),
        "DeathOperation 绕过了死亡前的救回阶段"
    );
}

#[derive(Debug)]
struct CountingAbility(Rc<Cell<usize>>);
impl Ability for CountingAbility {
    fn operation(&self, _: PlayerId, _: &GameState) -> Option<Box<dyn Operation>> {
        Some(Box::new(CountOperation(self.0.clone())))
    }
}

#[derive(Debug)]
struct EndOperation;
impl Operation for EndOperation {
    fn execute(
        self: Box<Self>,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        ctx.end_game(duel::core::GameResult::Draw)?;
        completed()
    }
}

#[derive(Debug)]
struct EndAtFirstActionCheckpoint;
impl System for EndAtFirstActionCheckpoint {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::Checkpoint), Priority::Default)]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        matches!(
            fact.event(),
            Some(Event::Checkpoint {
                phase: Checkpoint::ActionEnd,
                ..
            })
        )
        .then(|| vec![Subject::Standalone])
        .unwrap_or_default()
    }
    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(vec![Box::new(EndOperation)])
    }
}

#[test]
fn c4a_checkpoint_between_normal_abilities_stops_second_ability() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    world.add_player(Player::new("B".into(), 10, 0));
    let count = Rc::new(Cell::new(0));
    world.add_data(
        Some(a),
        Abilities::new(
            a,
            vec![
                Box::new(CountingAbility(count.clone())),
                Box::new(CountingAbility(count.clone())),
            ],
        ),
    );
    world.add_system(EndAtFirstActionCheckpoint);
    world.run_with_max_rounds(1).expect("对局应正常结束");
    assert_eq!(
        count.get(),
        1,
        "两个正常技能都在第一个 ActionEnd 检查点之前执行"
    );
}

#[test]
fn maximum_unsigned_damage_must_not_become_healing() {
    let mut world = World::new();
    let player = world.add_player(Player::new("A".into(), 10, 0));
    world.get_player_mut(player).expect("初始化玩家").set_hp(5);
    let (_, value) = world
        .execute(Damage::new(None, player, u64::MAX))
        .expect("伤害");
    let change = value
        .expect("生命结果")
        .downcast::<HpChange>()
        .expect("HpChange");
    assert_eq!(change.new_hp, 0, "u64::MAX 伤害变成了 +1 治疗");
}

#[test]
fn maximum_unsigned_heal_must_not_become_damage() {
    let mut world = World::new();
    let player = world.add_player(Player::new("A".into(), 10, 0));
    world.get_player_mut(player).expect("初始化玩家").set_hp(5);
    let (_, value) = world.execute(Heal::new(player, u64::MAX)).expect("治疗");
    let change = value
        .expect("生命结果")
        .downcast::<HpChange>()
        .expect("HpChange");
    assert_eq!(change.new_hp, 10, "u64::MAX 治疗变成了 -1 伤害");
}

#[derive(Debug)]
struct AttackDamageObserver(Rc<Cell<u64>>);
impl System for AttackDamageObserver {
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
fn after_attack_damage_should_match_post_reduction_damage_for_non_overkill() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 8));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    let reported = Rc::new(Cell::new(0));
    register_damage_reduction_rule(&mut world);
    add_damage_reduction(&mut world, b, b, 0.25);
    world.add_system(AttackDamageObserver(reported.clone()));
    world.execute(AttackOperation::new(a)).expect("攻击");
    assert_eq!(world.get_player(b).expect("B 存活").hp(), 14);
    assert_eq!(reported.get(), 6, "通知携带的是减伤前的数值");
}

// —— 设计审计表场景 ——

/// 一次性护盾：数据实例 + 业务规则。
/// 规则把伤害草稿降为 0 并声明消耗；消耗由受控提交统一处理。
#[derive(Debug)]
struct AuditShieldData {
    target_id: PlayerId,
}

#[derive(Debug)]
struct AuditShieldRule;

impl DamageRule for AuditShieldRule {
    fn candidates(
        &self,
        context: &DamageContext,
        query: &duel::core::query::Query<'_>,
    ) -> Vec<Subject> {
        query
            .instances::<AuditShieldData>()
            .iter()
            .filter(|(_, _, data)| data.target_id == context.target_id)
            .map(|(id, _, _)| Subject::Instance(*id))
            .collect()
    }

    fn modify(
        &self,
        context: &mut DamageContext,
        subject: Subject,
        query: &duel::core::query::Query<'_>,
    ) {
        let Subject::Instance(id) = subject else {
            return;
        };
        let Some(data) = query.data::<AuditShieldData>(id) else {
            return;
        };
        if data.target_id != context.target_id {
            return;
        }
        context.reduce_to(0);
        context.consume_buff(id);
    }
}

#[test]
fn audit_two_attacks_consume_one_shot_shield_exactly_once() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 3));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    register_damage_rule(&mut world, Priority::Modify, AuditShieldRule);
    let shield = world.add_data(Some(b), AuditShieldData { target_id: b });

    world.execute(AttackOperation::new(a)).expect("第一次攻击");
    assert_eq!(
        world.get_player(b).expect("B 存活").hp(),
        20,
        "护盾应挡下第一次攻击"
    );
    assert!(
        world.get_data::<AuditShieldData>(shield).is_none(),
        "护盾应在被挡下的这次提交中一并消耗"
    );

    // 第二次攻击独立结算，读取护盾消耗后的新状态。
    world.execute(AttackOperation::new(a)).expect("第二次攻击");
    assert_eq!(world.get_player(b).expect("B 存活").hp(), 17);
}

#[test]
fn audit_second_rescue_reads_updated_state_and_stays_available() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let player = world.add_player(Player::new("A".into(), 10, 0));
    register_revival_system(&mut world);
    let first = add_revival(&mut world, player);
    let second = add_revival(&mut world, player);

    world
        .execute(Damage::new(None, player, 10))
        .expect("致命伤害");
    assert_eq!(
        world.get_player(player).map(|p| p.hp()),
        Some(5),
        "第一个救回应以半血复活"
    );
    assert!(
        world.get_data::<RevivalData>(first).is_none(),
        "第一次救回的机会应被消耗"
    );
    assert!(
        world.get_data::<RevivalData>(second).is_some(),
        "已不需要救回时不得重复消耗第二个救回机会"
    );
}

#[derive(Debug)]
struct RemovePeer {
    peer: BuffId,
}

impl System for RemovePeer {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::Checkpoint), Priority::Modify)]
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
        Ok(vec![Box::new(RemoveDataOperation {
            buff_id: self.peer,
            reason: DestructionReason::Explicit,
        })])
    }
}

#[test]
fn audit_instance_removed_mid_dispatch_is_skipped_in_current_event() {
    let mut world = World::new();
    let count = Rc::new(Cell::new(0));
    let marker = world.add_data(None, MarkerData);
    world.add_system(MarkerCounter(count.clone()));
    world.add_system(RemovePeer { peer: marker });

    world.execute(EmitEvent(checkpoint())).expect("事件");
    assert_eq!(count.get(), 0, "轮到之前被移除的候选应被跳过");
}

/// 全局死亡爆炸：有角色死亡时对另一名角色造成 10 点伤害。
#[derive(Debug)]
struct DeathBlast {
    participants: Vec<PlayerId>,
}

impl System for DeathBlast {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(
            NoticeKind::Event(EventType::AfterPlayerDeath),
            Priority::Default,
        )]
    }

    fn candidates(&self, fact: &Fact<'_>, _query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        matches!(fact.event(), Some(Event::AfterPlayerDeath(_)))
            .then(|| vec![Subject::Standalone])
            .unwrap_or_default()
    }

    fn respond(
        &self,
        fact: &Fact<'_>,
        _subject: Subject,
        query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        let Some(Event::AfterPlayerDeath(dead)) = fact.event() else {
            return Ok(vec![]);
        };
        Ok(self
            .participants
            .iter()
            .filter(|&&other| other != *dead)
            .filter(|&&other| query.player(other).is_some_and(|p| p.is_alive()))
            .map(|&other| Box::new(Damage::new(None, other, 10)) as Box<dyn Operation>)
            .collect())
    }
}

#[test]
fn audit_death_reaction_kills_further_players_through_the_same_mechanism() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 10, 0));
    world.add_system(DeathBlast {
        participants: vec![a, b],
    });

    world
        .execute(Damage::new(None, a, 10))
        .expect("A 受致命伤害");
    assert!(world.get_player(a).is_none(), "A 应死亡");
    assert!(
        world.get_player(b).is_none(),
        "死亡反应应通过同一机制继续造成死亡，而不需要特殊分发"
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
