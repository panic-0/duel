//! 生命提交历史值、同步反应和无符号数值边界。

use duel::core::{
    event::{Event, EventKind},
    operation::{
        completed_with, ActionContext, HpChange, Operation, OperationError, OperationOutcome,
        OperationResult,
    },
    player::Player,
    system::{EventEnvelope, Priority, ReactionTarget, System},
    BattleEngine, Damage, Heal, PlayerId,
};

/// 受伤后回复 3 点的响应 System，用于检验提交返回前反应是否完成。
#[derive(Debug)]
struct HealAfterDamageEvent(PlayerId);

impl System for HealAfterDamageEvent {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::HpChanged, Priority::Default)]
    }

    fn candidates(
        &self,
        _fact: &EventEnvelope<'_>,
        _query: &duel::core::query::Query<'_>,
    ) -> Vec<ReactionTarget> {
        vec![ReactionTarget::Standalone]
    }

    fn respond(
        &self,
        fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
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
        context: &mut ActionContext<'_>,
    ) -> Result<(OperationResult, Option<Box<dyn std::any::Any>>), OperationError> {
        let change = context
            .modify_hp(self.0, -3)
            .expect("提交应成功")
            .expect("玩家应存在");
        Ok((OperationResult::Completed, Some(Box::new(change))))
    }
}

#[test]
fn hp_submission_returns_history_and_runs_reactions() {
    let mut world = BattleEngine::new();
    let player = world.add_player(Player::new("A".into(), 10, 1));
    world.register_system(HealAfterDamageEvent(player));
    let outcome = world.execute(DamageOnce(player)).expect("操作应成功");
    let value = outcome.1.expect("应返回生命结果");
    let change = value
        .downcast_ref::<HpChange>()
        .expect("类型应匹配")
        .to_owned();
    // 返回的是本次提交的历史事实（10 → 7）……
    assert_eq!((change.old_hp, change.new_hp), (10, 7));
    // ……而当前状态已包含反应（回复 3 点）。
    assert_eq!(world.player(player).expect("玩家应存在").hp(), 10);
}

#[derive(Debug)]
struct HealAfterHpChange(PlayerId);

impl System for HealAfterHpChange {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::HpChanged, Priority::Default)]
    }
    fn candidates(
        &self,
        _fact: &EventEnvelope<'_>,
        _query: &duel::core::query::Query<'_>,
    ) -> Vec<ReactionTarget> {
        vec![ReactionTarget::Standalone]
    }
    fn respond(
        &self,
        fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
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
        ctx: &mut ActionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        let change = ctx
            .modify_hp(self.0, -4)
            .expect("提交应成功")
            .expect("目标应存在");
        let now = ctx.state().player(self.0).expect("目标存活").hp();
        completed_with((change.new_hp, now))
    }
}

#[test]
fn hp_submission_returns_history_after_finishing_reactions() {
    let mut world = BattleEngine::new();
    let player = world.add_player(Player::new("A".into(), 10, 0));
    world.register_system(HealAfterHpChange(player));
    let (_, value) = world.execute(DamageThenRead(player)).expect("根操作");
    let observed = *value
        .expect("应读取到生命值")
        .downcast::<(u64, u64)>()
        .expect("元组");
    assert_eq!(observed, (6, 9), "历史值应为 6，当前值应包含治疗反应");
}

#[test]
fn maximum_unsigned_damage_must_not_become_healing() {
    let mut world = BattleEngine::new();
    let player = world.add_player(Player::new("A".into(), 10, 0));
    assert!(world.set_initial_hp(player, 5));
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
    let mut world = BattleEngine::new();
    let player = world.add_player(Player::new("A".into(), 10, 0));
    assert!(world.set_initial_hp(player, 5));
    let (_, value) = world.execute(Heal::new(player, u64::MAX)).expect("治疗");
    let change = value
        .expect("生命结果")
        .downcast::<HpChange>()
        .expect("HpChange");
    assert_eq!(change.new_hp, 10, "u64::MAX 治疗变成了 -1 伤害");
}
