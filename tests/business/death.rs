//! 死亡确认、重入保护、救回窗口与连锁死亡。

use crate::support::{op_system, trace_events, BelovedMark, Count, DependencyCounter};
use duel::core::{
    business::{
        revival::{attach_revival, register_revival_system},
        skills::Abilities,
    },
    event::{Event, EventKind},
    install_default_rules,
    log::{LogEntry, Logger},
    operation::{AddPlayerOperation, Operation, OperationError},
    player::Player,
    query::Query,
    system::{EventEnvelope, Priority, ReactionTarget, System},
    Attack, BattleEngine, Damage, DeathOperation, DuelRunner, Heal, PlayerId,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// 自定义死亡判断：观察到零血角色的生命变化后提出 Death。
#[derive(Debug)]
struct GlobalDeathRule;

impl System for GlobalDeathRule {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::HpChanged, Priority::Default)]
    }
    fn candidates(
        &self,
        fact: &EventEnvelope<'_>,
        query: &duel::core::query::Query<'_>,
    ) -> Vec<ReactionTarget> {
        if !matches!(fact.event(), Some(Event::HpChanged { .. })) {
            return Vec::new();
        }
        if query.state().players().values().any(|p| p.hp() == 0) {
            vec![ReactionTarget::Standalone]
        } else {
            Vec::new()
        }
    }
    fn respond(
        &self,
        _fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(query
            .state()
            .players()
            .iter()
            .filter(|(_, p)| p.hp() == 0)
            .map(|(&id, _)| Box::new(DeathOperation { player_id: id }) as Box<dyn Operation>)
            .collect())
    }
}

#[test]
fn ordinary_death_operation_preserves_before_death_rescue_when_triggered_by_global_rule() {
    let mut world = BattleEngine::new();
    let player = world.add_player(Player::new("A".into(), 10, 0));
    world.register_system(GlobalDeathRule);
    register_revival_system(&mut world);
    attach_revival(&mut world, player);
    world
        .execute(Damage::new(None, player, 10))
        .expect("致命伤害");
    assert_eq!(
        world.player(player).map(|p| p.hp()),
        Some(5),
        "DeathOperation 绕过了死亡前的救回阶段"
    );
}

/// 全局死亡爆炸：有角色死亡时对另一名角色造成 10 点伤害。
#[derive(Debug)]
struct DeathBlast {
    participants: Vec<PlayerId>,
}

impl System for DeathBlast {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::AfterPlayerDeath, Priority::Default)]
    }

    fn candidates(
        &self,
        fact: &EventEnvelope<'_>,
        _query: &duel::core::query::Query<'_>,
    ) -> Vec<ReactionTarget> {
        matches!(fact.event(), Some(Event::AfterPlayerDeath(_)))
            .then(|| vec![ReactionTarget::Standalone])
            .unwrap_or_default()
    }

    fn respond(
        &self,
        fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
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
fn death_reaction_kills_further_players_through_the_same_mechanism() {
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 10, 0));
    world.register_system(DeathBlast {
        participants: vec![a, b],
    });

    world
        .execute(Damage::new(None, a, 10))
        .expect("A 受致命伤害");
    assert!(world.player(a).is_none(), "A 应死亡");
    assert!(
        world.player(b).is_none(),
        "死亡反应应通过同一机制继续造成死亡，而不需要特殊分发"
    );
}

/// 统计死亡前通知次数的探针。
#[derive(Debug)]
struct BeforeDeathCounter {
    priority: Priority,
    count: Rc<Cell<usize>>,
}

impl System for BeforeDeathCounter {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::BeforePlayerDeath, self.priority)]
    }
    fn candidates(
        &self,
        fact: &EventEnvelope<'_>,
        _query: &duel::core::query::Query<'_>,
    ) -> Vec<ReactionTarget> {
        matches!(fact.event(), Some(Event::BeforePlayerDeath(_)))
            .then(|| vec![ReactionTarget::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        _fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
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
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::BeforePlayerDeath, Priority::Default)]
    }
    fn candidates(
        &self,
        fact: &EventEnvelope<'_>,
        _query: &duel::core::query::Query<'_>,
    ) -> Vec<ReactionTarget> {
        matches!(fact.event(), Some(Event::BeforePlayerDeath(_)))
            .then(|| vec![ReactionTarget::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        _fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(vec![Box::new(DeathOperation {
            player_id: self.player,
        })])
    }
}

#[test]
fn reentering_the_same_pending_death_does_not_repeat_before_death_effects() {
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let count = Rc::new(Cell::new(0));
    world.register_system(BeforeDeathCounter {
        priority: Priority::Modify,
        count: count.clone(),
    });
    world.register_system(RepeatSameDeath { player: a });

    world
        .execute(Damage::new(None, a, 10))
        .expect("有限的死亡链");

    assert!(world.player(a).is_none());
    assert_eq!(
        count.get(),
        1,
        "同一次正在进行的死亡不能重复执行死亡前副作用"
    );
}

#[test]
fn the_dispatcher_does_not_suppress_remaining_before_death_hooks_after_rescue() {
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let count = Rc::new(Cell::new(0));
    register_revival_system(&mut world);
    attach_revival(&mut world, a);
    world.register_system(BeforeDeathCounter {
        priority: Priority::Final,
        count: count.clone(),
    });

    world.execute(Damage::new(None, a, 10)).expect("救回");

    assert_eq!(world.player(a).unwrap().hp(), 5);
    assert_eq!(
        count.get(),
        1,
        "候选仍存在；应由 System 自己判断新状态，而非 BattleEngine 特判跳过整段死亡前通知"
    );
}

#[test]
fn custom_death_rule_without_default_rules_completes_full_lifecycle() {
    let mut world = BattleEngine::new();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 10, 0));
    world.attach_component(Some(a), BelovedMark);
    // 只装配自定义死亡判断，不装配默认规则。
    world.register_system(CustomDeathSystem);
    let after_death = Rc::new(Cell::new(0));
    world.register_system(DependencyCounter {
        after_death_count: after_death.clone(),
        observed_instances: Rc::new(Cell::new(-1i64)),
    });

    world
        .execute(Damage::new(None, a, 10))
        .expect("致命伤害应正常结算");

    assert!(world.player(a).is_none(), "自定义死亡应完成角色移除");
    assert!(
        world.query().components::<BelovedMark>().is_empty(),
        "owner 依赖销毁由受控提交完成"
    );
    assert_eq!(after_death.get(), 1, "死亡后通知应完整发布");
    assert!(world.player(b).is_some());
    assert!(!world.is_end(), "未装配胜负规则时不得自行判负");
}

#[derive(Debug)]
struct CustomDeathSystem;

impl System for CustomDeathSystem {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::HpChanged, Priority::Final)]
    }
    fn candidates(&self, fact: &EventEnvelope<'_>, query: &Query<'_>) -> Vec<ReactionTarget> {
        if !matches!(fact.event(), Some(Event::HpChanged { .. })) {
            return Vec::new();
        }
        if query.state().players().values().any(|p| p.hp() == 0) {
            vec![ReactionTarget::Standalone]
        } else {
            Vec::new()
        }
    }
    fn respond(
        &self,
        _fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(query
            .state()
            .players()
            .iter()
            .filter(|(_, p)| p.hp() == 0)
            .map(|(&id, _)| Box::new(DeathOperation { player_id: id }) as Box<dyn Operation>)
            .collect())
    }
}

#[test]
fn death_summon_finishes_before_last_survivor_check() {
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    world.add_player(Player::new("攻击者".into(), 10, 10));
    let target = world.add_player(Player::new("召唤者".into(), 10, 1));
    world.attach_component(Some(0), Abilities::new(0, vec![Box::new(Attack)]));
    op_system(&mut world, &[EventKind::AfterPlayerDeath], |_, _| {
        vec![
            Box::new(AddPlayerOperation(Player::new("召唤物".into(), 10, 1))) as Box<dyn Operation>,
        ]
    });
    // 召唤发生在死亡通知内、下一个检查点之前，
    // 因此“只剩一人”的胜负判断始终不成立，对局一直有新对手。
    world.run_with_max_rounds(4).expect("对局应正常结束");
    assert!(world.player(target).is_none());
    assert_eq!(world.players().len(), 2, "召唤物应接替死亡者，使对局持续");
    assert!(world.is_end(), "到达回合上限后以平局结束");
}

#[test]
fn duplicate_death_requests_emit_one_notification_and_log() {
    let mut world = BattleEngine::new();
    let a = world.add_player(Player::new("A".into(), 10, 1));
    let trace = trace_events(&mut world);
    let logs = Rc::new(RefCell::new(Vec::new()));
    let sink = logs.clone();
    world.set_logger(Logger::new(Box::new(move |_, entry| {
        sink.borrow_mut().push(entry.clone())
    })));
    // 同一响应提出两次死亡请求：防重入应让第二次跳过。
    op_system(&mut world, &[EventKind::HpChanged], move |fact, state| {
        if !matches!(fact.event(), Some(Event::HpChanged { .. })) {
            return vec![];
        }
        state
            .players()
            .iter()
            .filter(|(_, player)| player.hp() == 0)
            .flat_map(|(&id, _)| {
                let first: Box<dyn Operation> =
                    Box::new(duel::core::DeathOperation { player_id: id });
                let second: Box<dyn Operation> =
                    Box::new(duel::core::DeathOperation { player_id: id });
                vec![first, second]
            })
            .collect()
    });
    world
        .execute(Damage::new(None, a, 10))
        .expect("致命伤害应正常结算");
    assert_eq!(
        trace
            .borrow()
            .iter()
            .filter(|e| **e == Event::BeforePlayerDeath(a))
            .count(),
        1,
        "重复的死亡请求只应产生一次死亡前通知"
    );
    assert_eq!(
        trace
            .borrow()
            .iter()
            .filter(|e| **e == Event::AfterPlayerDeath(a))
            .count(),
        1
    );
    assert_eq!(
        logs.borrow()
            .iter()
            .filter(|e| **e == LogEntry::Death { player_id: a })
            .count(),
        1,
        "重复的死亡请求只应记录一次死亡日志"
    );
    assert!(world.player(a).is_none());
}

#[test]
fn rescue_during_before_death_prevents_death_notifications() {
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    let hero = world.add_player(Player::new("英雄".into(), 10, 1));
    let deaths = Rc::new(RefCell::new(0));
    let sink = deaths.clone();
    op_system(&mut world, &[EventKind::BeforePlayerDeath], move |_, _| {
        vec![Box::new(Heal::new(hero, 5)) as Box<dyn Operation>]
    });
    op_system(&mut world, &[EventKind::AfterPlayerDeath], move |_, _| {
        *sink.borrow_mut() += 1;
        vec![]
    });
    world
        .execute(Damage::new(None, hero, 10))
        .expect("致命伤害应正常结算");
    assert_eq!(world.player(hero).map(|p| p.hp()), Some(5));
    assert_eq!(*deaths.borrow(), 0, "救回成功后不应有死亡后通知");
}
