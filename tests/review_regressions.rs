//! 第一轮审查回归测试：断言定稿协议（N1A/C1A/C3A/C4A、错误传播、归属过滤、
//! 数值方向）以及设计审计表的关键场景。随仓库一起编译并参与 `cargo test`。

use duel::core::{
    ability::{Abilities, Ability, AttackOperation},
    buff::{Buff, DamageReduction, Priority, Revival},
    event::{Checkpoint, Event, EventType},
    operation::{
        completed, completed_with, Damage, DamageContext, DeathOperation, EmitEvent,
        ExecutionContext, Heal, HpChange, Operation, OperationError, OperationOutcome,
        RemoveBuffOperation,
    },
    player::Player,
    state::GameState,
    world::World,
    BuffId, PlayerId,
};
use std::{cell::Cell, rc::Rc};

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
impl Buff for CheckpointCounter {
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        vec![(EventType::Checkpoint, Priority::Default)]
    }
    fn operations(&mut self, _: &Event, _: &GameState, _: BuffId) -> Vec<Box<dyn Operation>> {
        vec![Box::new(CountOperation(self.0.clone()))]
    }
}

#[derive(Debug)]
struct PublishThenRead(Rc<Cell<usize>>);
impl Operation for PublishThenRead {
    fn execute(
        self: Box<Self>,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        ctx.publish(checkpoint());
        completed_with(self.0.get())
    }
}

#[test]
fn n1a_publish_finishes_reactions_before_returning_to_operation() {
    let mut world = World::new();
    let count = Rc::new(Cell::new(0));
    world.add_buff(Box::new(CheckpointCounter(count.clone())));
    let (_, value) = world.execute(PublishThenRead(count)).expect("根操作");
    let observed = *value
        .expect("应读取到计数")
        .downcast::<usize>()
        .expect("usize");
    assert_eq!(observed, 1, "publish 在反应运行之前就返回了");
}

#[derive(Debug)]
struct HealAfterDamage(PlayerId);
impl Buff for HealAfterDamage {
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        vec![(EventType::HpChanged, Priority::Default)]
    }
    fn operations(&mut self, event: &Event, _: &GameState, _: BuffId) -> Vec<Box<dyn Operation>> {
        match event {
            Event::HpChanged {
                target_id,
                old_hp,
                new_hp,
            } if *target_id == self.0 && new_hp < old_hp => vec![Box::new(Heal::new(self.0, 3))],
            _ => vec![],
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
    world.add_buff(Box::new(HealAfterDamage(player)));
    let (_, value) = world.execute(DamageThenRead(player)).expect("根操作");
    let observed = *value
        .expect("应读取到生命值")
        .downcast::<(u64, u64)>()
        .expect("元组");
    assert_eq!(observed, (6, 9), "历史值应为 6，当前值应包含治疗反应");
}

#[derive(Debug)]
struct PublishThenAdd(Rc<Cell<usize>>);
impl Operation for PublishThenAdd {
    fn execute(
        self: Box<Self>,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        ctx.publish(checkpoint());
        ctx.add_buff(Box::new(CheckpointCounter(self.0.clone())));
        completed()
    }
}

#[test]
fn n1a_and_c3a_later_added_buff_does_not_receive_previously_published_event() {
    let mut world = World::new();
    let count = Rc::new(Cell::new(0));
    world
        .execute(PublishThenAdd(count.clone()))
        .expect("根操作");
    assert_eq!(count.get(), 0, "新增 Buff 收到了在它创建之前发布的事件");
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
struct FailingBuff;
impl Buff for FailingBuff {
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        vec![(EventType::Checkpoint, Priority::Default)]
    }
    fn operations(&mut self, _: &Event, _: &GameState, _: BuffId) -> Vec<Box<dyn Operation>> {
        vec![Box::new(FailOperation)]
    }
}

#[test]
fn inline_operation_error_reaches_root_and_stops_later_listeners() {
    let mut world = World::new();
    let count = Rc::new(Cell::new(0));
    world.add_buff(Box::new(FailingBuff));
    world.add_buff(Box::new(CheckpointCounter(count.clone())));
    let result = world.execute(EmitEvent(checkpoint()));
    assert!(result.is_err(), "嵌套 Err 被静默忽略");
    assert_eq!(count.get(), 0, "操作失败后更晚的监听者仍然运行了");
}

#[derive(Debug)]
struct OwnedHpCounter {
    owner: PlayerId,
    count: Rc<Cell<usize>>,
}
impl Buff for OwnedHpCounter {
    fn owner(&self) -> Option<PlayerId> {
        Some(self.owner)
    }
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        vec![(EventType::HpChanged, Priority::Default)]
    }
    fn operations(&mut self, _: &Event, _: &GameState, _: BuffId) -> Vec<Box<dyn Operation>> {
        vec![Box::new(CountOperation(self.count.clone()))]
    }
}

#[test]
fn owner_scope_filters_single_target_hp_events() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 10, 0));
    let count = Rc::new(Cell::new(0));
    world.add_buff(Box::new(OwnedHpCounter {
        owner: a,
        count: count.clone(),
    }));
    world.execute(Damage::new(None, b, 1)).expect("伤害");
    assert_eq!(count.get(), 0, "归属 A 的监听者收到了 B 的生命事件");
}

#[derive(Debug)]
struct GlobalDeathRule;
impl Buff for GlobalDeathRule {
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        vec![(EventType::HpChanged, Priority::Default)]
    }
    fn operations(
        &mut self,
        event: &Event,
        state: &GameState,
        _: BuffId,
    ) -> Vec<Box<dyn Operation>> {
        match event {
            Event::HpChanged { target_id, .. }
                if state.get_player(*target_id).is_some_and(|p| p.hp() == 0) =>
            {
                vec![Box::new(DeathOperation {
                    player_id: *target_id,
                })]
            }
            _ => vec![],
        }
    }
}

#[test]
fn ordinary_death_operation_preserves_before_death_rescue_when_triggered_by_global_rule() {
    let mut world = World::new();
    let player = world.add_player(Player::new("A".into(), 10, 0));
    world.add_buff(Box::new(GlobalDeathRule));
    world.add_buff(Box::new(Revival::new(player)));
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
        ctx.end_game(duel::core::flow::GameResult::Draw);
        completed()
    }
}

#[derive(Debug)]
struct EndAtFirstActionCheckpoint;
impl Buff for EndAtFirstActionCheckpoint {
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        vec![(EventType::Checkpoint, Priority::Default)]
    }
    fn operations(&mut self, event: &Event, _: &GameState, _: BuffId) -> Vec<Box<dyn Operation>> {
        if matches!(
            event,
            Event::Checkpoint {
                phase: Checkpoint::ActionEnd,
                ..
            }
        ) {
            vec![Box::new(EndOperation)]
        } else {
            vec![]
        }
    }
}

#[test]
fn c4a_checkpoint_between_normal_abilities_stops_second_ability() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    world.add_player(Player::new("B".into(), 10, 0));
    let count = Rc::new(Cell::new(0));
    world.add_buff(Box::new(Abilities::new(
        a,
        vec![
            Box::new(CountingAbility(count.clone())),
            Box::new(CountingAbility(count.clone())),
        ],
    )));
    world.add_buff(Box::new(EndAtFirstActionCheckpoint));
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
impl Buff for AttackDamageObserver {
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        vec![(EventType::AfterPlayerAttack, Priority::Default)]
    }
    fn operations(&mut self, event: &Event, _: &GameState, _: BuffId) -> Vec<Box<dyn Operation>> {
        if let Event::AfterPlayerAttack { damage, .. } = event {
            self.0.set(*damage);
        }
        vec![]
    }
}

#[test]
fn after_attack_damage_should_match_post_reduction_damage_for_non_overkill() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 8));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    let reported = Rc::new(Cell::new(0));
    world.add_buff(Box::new(DamageReduction::new(b, 0.25)));
    world.add_buff(Box::new(AttackDamageObserver(reported.clone())));
    world.execute(AttackOperation::new(a)).expect("攻击");
    assert_eq!(world.get_player(b).expect("B 存活").hp(), 14);
    assert_eq!(reported.get(), 6, "通知携带的是减伤前的数值");
}

// —— 设计审计表场景 ——

/// 一次性护盾：修改草稿时把伤害降为 0，并声明本次提交消耗自身。
/// 回调只拿到 `&self`，消耗由受控提交统一处理。
#[derive(Debug)]
struct OneShotShield {
    target_id: PlayerId,
}

impl Buff for OneShotShield {
    fn owner(&self) -> Option<PlayerId> {
        Some(self.target_id)
    }

    fn damage_modification(&self) -> Option<Priority> {
        Some(Priority::Modify)
    }

    fn modify_damage(&self, context: &mut DamageContext, _world: &GameState, buff_id: BuffId) {
        if context.target_id != self.target_id {
            return;
        }
        context.reduce_to(0);
        context.consume_buff(buff_id);
    }
}

#[test]
fn audit_two_attacks_consume_one_shot_shield_exactly_once() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 3));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    world.add_buff(Box::new(OneShotShield { target_id: b }));

    world.execute(AttackOperation::new(a)).expect("第一次攻击");
    assert_eq!(
        world.get_player(b).expect("B 存活").hp(),
        20,
        "护盾应挡下第一次攻击"
    );
    assert!(
        world
            .get_buffs()
            .values()
            .all(|buff| buff.damage_modification().is_none()),
        "护盾应在被挡下的这次提交中一并消耗"
    );

    // 第二次攻击独立结算，读取护盾消耗后的新状态。
    world.execute(AttackOperation::new(a)).expect("第二次攻击");
    assert_eq!(world.get_player(b).expect("B 存活").hp(), 17);
}

#[test]
fn audit_second_rescue_reads_updated_state_and_stays_available() {
    let mut world = World::new();
    world.install_default_rules();
    let player = world.add_player(Player::new("A".into(), 10, 0));
    world.add_buff(Box::new(Revival::new(player)));
    world.add_buff(Box::new(Revival::new(player)));

    world
        .execute(Damage::new(None, player, 10))
        .expect("致命伤害");
    assert_eq!(
        world.get_player(player).map(|p| p.hp()),
        Some(5),
        "第一个救回应以半血复活"
    );
    // Buff：默认死亡/胜负规则 + 尚未消耗的第二个救回。
    assert_eq!(
        world.get_buffs().len(),
        3,
        "已不需要救回时不得重复消耗第二个救回机会"
    );
}

#[derive(Debug)]
struct RemovePeer {
    peer: BuffId,
}

impl Buff for RemovePeer {
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        vec![(EventType::Checkpoint, Priority::Modify)]
    }

    fn operations(
        &mut self,
        _event: &Event,
        _world: &GameState,
        _buff_id: BuffId,
    ) -> Vec<Box<dyn Operation>> {
        vec![Box::new(RemoveBuffOperation(self.peer))]
    }
}

#[test]
fn audit_buff_removed_mid_dispatch_is_skipped_in_current_event() {
    let mut world = World::new();
    let count = Rc::new(Cell::new(0));
    let counter = world.add_buff(Box::new(CheckpointCounter(count.clone())));
    world.add_buff(Box::new(RemovePeer { peer: counter }));

    world.execute(EmitEvent(checkpoint())).expect("事件");
    assert_eq!(count.get(), 0, "轮到之前被移除的候选应被跳过");
}

/// 全局死亡爆炸：有角色死亡时对另一名角色造成 10 点伤害。
#[derive(Debug)]
struct DeathBlast {
    participants: Vec<PlayerId>,
}

impl Buff for DeathBlast {
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        vec![(EventType::AfterPlayerDeath, Priority::Default)]
    }

    fn operations(
        &mut self,
        event: &Event,
        state: &GameState,
        _buff_id: BuffId,
    ) -> Vec<Box<dyn Operation>> {
        let Event::AfterPlayerDeath(dead) = *event else {
            return vec![];
        };
        self.participants
            .iter()
            .filter(|&&other| other != dead)
            .filter(|&&other| state.get_player(other).is_some_and(|p| p.is_alive()))
            .map(|&other| Box::new(Damage::new(None, other, 10)) as Box<dyn Operation>)
            .collect()
    }
}

#[test]
fn audit_death_reaction_kills_further_players_through_the_same_mechanism() {
    let mut world = World::new();
    world.install_default_rules();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 10, 0));
    world.add_buff(Box::new(DeathBlast {
        participants: vec![a, b],
    }));

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

impl Buff for ChainDriver {
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        vec![(EventType::RoundEnd, Priority::Default)]
    }

    fn operations(
        &mut self,
        event: &Event,
        _world: &GameState,
        _buff_id: BuffId,
    ) -> Vec<Box<dyn Operation>> {
        match *event {
            Event::RoundEnd { round } => vec![Box::new(PublishChain(round))],
            _ => vec![],
        }
    }
}

#[test]
fn audit_runaway_reaction_chain_is_bounded_instead_of_overflowing() {
    let mut world = World::new();
    world.add_buff(Box::new(ChainDriver));
    let result = world.execute(PublishChain(0));
    assert!(result.is_err(), "失控反应链应报错终止，而不是栈溢出");
    assert!(world.is_operation_failed(), "深度上限错误应记录到世界");
}
