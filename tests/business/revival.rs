//! 复活次数、关联消费与业务目标匹配。

use crate::support::{count, log_sink};
use duel::core::{
    business::{
        revival::{attach_revival, register_revival_system, RevivalData},
        skills::Abilities,
    },
    component::DestructionReason,
    event::{Event, EventKind},
    install_default_rules,
    log::LogEntry,
    operation::{completed, ActionContext, EmitEvent, Operation, OperationError, OperationOutcome},
    player::Player,
    query::Query,
    system::{Destruction, EventEnvelope, Priority, ReactionTarget, System},
    Attack, BattleEngine, ComponentId, Damage, DuelRunner, PlayerId,
};
use std::any::Any;

#[test]
fn revival_triggers_exactly_once() {
    let (entries, logger) = log_sink();
    let mut world = BattleEngine::new();
    world.set_logger(logger);

    let p1 = world.add_player(Player::new("Hero".to_string(), 10, 2));
    let p2 = world.add_player(Player::new("Boss".to_string(), 100, 10));
    world.attach_component(Some(p1), Abilities::new(p1, vec![Box::new(Attack)]));
    register_revival_system(&mut world);
    attach_revival(&mut world, p1);
    world.attach_component(Some(p2), Abilities::new(p2, vec![Box::new(Attack)]));
    install_default_rules(&mut world);

    world.run().expect("对局应正常结束");

    let entries = entries.borrow();
    assert_eq!(
        count(&entries, |e| matches!(e, LogEntry::Revival { .. })),
        1,
        "复活应恰好触发一次"
    );
    assert_eq!(
        count(&entries, |e| matches!(e, LogEntry::Death { .. })),
        1,
        "第二次致死伤害应直接死亡"
    );
    assert!(world.is_end());
    assert!(world.player(p1).is_none());
    assert_eq!(world.player(p2).unwrap().hp(), 96);
}

/// 观察到复活机会被 Consumed 时，对其服务对象追加 7 点伤害的 System。
#[derive(Debug)]
struct ConsumeRetaliator {
    damage: u64,
}

impl System for ConsumeRetaliator {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::ComponentDestroyed, Priority::Default)]
    }
    fn candidates(&self, fact: &EventEnvelope<'_>, _query: &Query<'_>) -> Vec<ReactionTarget> {
        matches!(fact, EventEnvelope::Destroyed(d) if d.reason == DestructionReason::Consumed)
            .then(|| vec![ReactionTarget::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        _query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        let EventEnvelope::Destroyed(Destruction { data, .. }) = fact else {
            return Ok(vec![]);
        };
        let data: &(dyn Any + 'static) = *data;
        let Some(revival) = duel::core::component::downcast_component::<RevivalData>(data) else {
            return Ok(vec![]);
        };
        Ok(vec![Box::new(Damage::new(
            None,
            revival.player_id,
            self.damage,
        ))])
    }
}

#[test]
fn revival_consumption_and_heal_are_one_associated_submission() {
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 20, 0));
    register_revival_system(&mut world);
    attach_revival(&mut world, a);
    world.register_system(ConsumeRetaliator { damage: 7 });

    world
        .execute(Damage::new(None, a, 20))
        .expect("致命伤害应触发救回");

    // 关联提交：消费与恢复一起完成后才开放通知 → 消费反应中的 7 点伤害落在 10 上。
    assert_eq!(
        world.player(a).map(|p| p.hp()),
        Some(3),
        "响应不得看到“机会已消耗但生命仍为零”的半次提交"
    );
    assert!(world.player(a).is_some(), "救回后不应死亡");
}

#[test]
fn revival_matches_by_business_player_id_regardless_of_owner() {
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    register_revival_system(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    // owner=A，服务对象=B：跨角色的救回机会。
    world.attach_component(Some(a), RevivalData { player_id: b });

    world
        .execute(Damage::new(None, b, 20))
        .expect("致命伤害应触发救回");

    assert_eq!(
        world.player(b).map(|p| p.hp()),
        Some(10),
        "救回应按 player_id 匹配，即使 owner 不是被救角色"
    );
    assert!(world.player(a).is_some());

    // owner=A 先死亡：救回机会随之销毁，之后 B 致死时不再有救回。
    world
        .execute(Damage::new(None, a, 10))
        .expect("A 应正常死亡");
    world
        .execute(Damage::new(None, b, 20))
        .expect("第二次致命伤害");
    assert!(
        world.player(b).is_none(),
        "owner 死亡后救回机会已被销毁，B 应正常死亡"
    );
}

#[test]
fn ownerless_revival_still_rescues_by_player_id() {
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    register_revival_system(&mut world);
    let b = world.add_player(Player::new("B".into(), 20, 0));
    world.attach_component(None, RevivalData { player_id: b });

    world
        .execute(Damage::new(None, b, 20))
        .expect("致命伤害应触发救回");

    assert_eq!(
        world.player(b).map(|p| p.hp()),
        Some(10),
        "owner=None 的救回数据不受任何角色死亡影响，按 player_id 正常救回"
    );
}

#[test]
fn second_rescue_reads_updated_state_and_stays_available() {
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    let player = world.add_player(Player::new("A".into(), 10, 0));
    register_revival_system(&mut world);
    let first = attach_revival(&mut world, player);
    let second = attach_revival(&mut world, player);

    world
        .execute(Damage::new(None, player, 10))
        .expect("致命伤害");
    assert_eq!(
        world.player(player).map(|p| p.hp()),
        Some(5),
        "第一个救回应以半血复活"
    );
    assert!(
        world.component::<RevivalData>(first).is_none(),
        "第一次救回的机会应被消耗"
    );
    assert!(
        world.component::<RevivalData>(second).is_some(),
        "已不需要救回时不得重复消耗第二个救回机会"
    );
}

#[derive(Debug)]
struct UpdateRevivalTarget {
    id: ComponentId,
    target: PlayerId,
}

impl Operation for UpdateRevivalTarget {
    fn execute(
        self: Box<Self>,
        context: &mut ActionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.update_component(
            self.id,
            RevivalData {
                player_id: self.target,
            },
        )?;
        completed()
    }
}

#[derive(Debug)]
struct RetargetBeforeRevival {
    event_player: PlayerId,
    revival: ComponentId,
    new_target: PlayerId,
}

impl System for RetargetBeforeRevival {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::BeforePlayerDeath, Priority::Modify)]
    }

    fn candidates(&self, fact: &EventEnvelope<'_>, _query: &Query<'_>) -> Vec<ReactionTarget> {
        if matches!(fact.event(), Some(Event::BeforePlayerDeath(id)) if *id == self.event_player) {
            vec![ReactionTarget::Standalone]
        } else {
            Vec::new()
        }
    }

    fn respond(
        &self,
        _fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        _query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(vec![Box::new(UpdateRevivalTarget {
            id: self.revival,
            target: self.new_target,
        })])
    }
}

#[test]
fn revival_rechecks_event_target_after_same_id_update() {
    let mut world = BattleEngine::new();
    let b = world.add_player(Player::new("B".into(), 10, 0));
    let c = world.add_player(Player::new("C".into(), 10, 0));
    // 初始化两名零血角色；不装配默认死亡规则，本例只测试这条通知的匹配。
    assert!(world.set_initial_hp(b, 0));
    assert!(world.set_initial_hp(c, 0));
    let revival = world.attach_component(None, RevivalData { player_id: b });
    world.register_system(RetargetBeforeRevival {
        event_player: b,
        revival,
        new_target: c,
    });
    register_revival_system(&mut world);

    world
        .execute(EmitEvent(Event::BeforePlayerDeath(b)))
        .expect("事件应正常分发");

    assert_eq!(world.player(b).unwrap().hp(), 0);
    assert_eq!(
        world.player(c).unwrap().hp(),
        0,
        "B 的死亡前通知不能因为候选数据被更新，而错误地救回 C"
    );
    assert_eq!(
        world
            .component::<RevivalData>(revival)
            .map(|data| data.player_id),
        Some(c),
        "更新可以成立，但不再匹配当前事件的机会应保留且不消费"
    );
}
