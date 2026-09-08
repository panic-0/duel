//! 攻击资格复查、伤害结果与攻击通知时序。

use crate::support::{op_system, trace_events};
use duel::core::{
    business::{
        damage_reduction::{attach_damage_reduction, register_damage_reduction_rule},
        skills::Abilities,
    },
    event::{Event, EventKind},
    install_default_rules,
    operation::{EmitEvent, Operation, OperationError, RemovePlayerOperation},
    player::Player,
    system::{EventEnvelope, Priority, ReactionTarget, System},
    Attack, AttackOperation, BattleEngine, Damage, DuelRunner, OperationResult, PlayerId,
};
use std::{cell::Cell, rc::Rc};

/// 陷阱：攻击开始事件对来源反噬 10 点伤害。
#[derive(Debug)]
struct Trap(PlayerId);

impl System for Trap {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::BeforePlayerAttack, Priority::Default)]
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
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    let source = world.add_player(Player::new("A".into(), 10, 4));
    let target = world.add_player(Player::new("B".into(), 10, 1));
    world.register_system(Trap(source));
    let result = world.execute(AttackOperation::new(source));
    assert!(result.is_ok());
    // 陷阱先杀死来源；攻击复查后放弃，不得对目标盲出伤害。
    assert!(world.player(source).is_none(), "来源应死于陷阱");
    assert_eq!(
        world.player(target).expect("目标应存在").hp(),
        10,
        "攻击在反应后复查来源，未继续提交伤害"
    );
}

#[derive(Debug)]
struct AttackDamageObserver(Rc<Cell<u64>>);

impl System for AttackDamageObserver {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::AfterPlayerAttack, Priority::Default)]
    }
    fn candidates(
        &self,
        fact: &EventEnvelope<'_>,
        _query: &duel::core::query::Query<'_>,
    ) -> Vec<ReactionTarget> {
        matches!(fact.event(), Some(Event::AfterPlayerAttack { .. }))
            .then(|| vec![ReactionTarget::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
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
    let mut world = BattleEngine::new();
    let a = world.add_player(Player::new("A".into(), 10, 8));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    let reported = Rc::new(Cell::new(0));
    register_damage_reduction_rule(&mut world);
    attach_damage_reduction(&mut world, b, b, 0.25);
    world.register_system(AttackDamageObserver(reported.clone()));
    world.execute(AttackOperation::new(a)).expect("攻击");
    assert_eq!(world.player(b).expect("B 存活").hp(), 14);
    assert_eq!(reported.get(), 6, "通知携带的是减伤前的数值");
}

/// PlayerAttack 响应中杀死攻击者。
#[derive(Debug)]
struct KillSourceAtPlayerAttack;

impl System for KillSourceAtPlayerAttack {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::PlayerAttack, Priority::Default)]
    }
    fn candidates(
        &self,
        fact: &EventEnvelope<'_>,
        _query: &duel::core::query::Query<'_>,
    ) -> Vec<ReactionTarget> {
        matches!(fact.event(), Some(Event::PlayerAttack { .. }))
            .then(|| vec![ReactionTarget::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
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
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 3));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    world.register_system(KillSourceAtPlayerAttack);

    world
        .execute(AttackOperation::new(a))
        .expect("攻击应正常结算");

    assert!(world.player(a).is_none());
    assert_eq!(
        world.player(b).unwrap().hp(),
        20,
        "采用默认普攻规则：来源在伤害提交前失效则不再扣血"
    );
}

/// PlayerAttack 响应中直接移除目标。
#[derive(Debug)]
struct RemoveTargetAtPlayerAttack;

impl System for RemoveTargetAtPlayerAttack {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::PlayerAttack, Priority::Default)]
    }
    fn candidates(
        &self,
        fact: &EventEnvelope<'_>,
        _query: &duel::core::query::Query<'_>,
    ) -> Vec<ReactionTarget> {
        matches!(fact.event(), Some(Event::PlayerAttack { .. }))
            .then(|| vec![ReactionTarget::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
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
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::AfterPlayerAttack, Priority::Default)]
    }
    fn candidates(
        &self,
        fact: &EventEnvelope<'_>,
        _query: &duel::core::query::Query<'_>,
    ) -> Vec<ReactionTarget> {
        matches!(fact.event(), Some(Event::AfterPlayerAttack { .. }))
            .then(|| vec![ReactionTarget::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
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
    let mut world = BattleEngine::new();
    let a = world.add_player(Player::new("A".into(), 10, 3));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    let reported = Rc::new(Cell::new(0));
    world.register_system(RemoveTargetAtPlayerAttack);
    world.register_system(ObserveAfterAttackDamage(reported.clone()));

    let (result, _) = world
        .execute(AttackOperation::new(a))
        .expect("目标失效应按跳过处理");

    assert!(world.player(b).is_none());
    assert_eq!(result, OperationResult::Skipped);
    assert_eq!(
        reported.get(),
        0,
        "Damage 没有提交时不得伪造正伤害通知，也不能回退到原始伤害"
    );
}

#[test]
fn fatal_attack_completes_notifications_and_retaliation_before_ending() {
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 10));
    let b = world.add_player(Player::new("B".into(), 10, 1));
    world.attach_component(Some(a), Abilities::new(a, vec![Box::new(Attack)]));
    let trace = trace_events(&mut world);
    let victim = b;
    let avenger = a;
    op_system(
        &mut world,
        &[EventKind::AfterPlayerDeath],
        move |fact, _| {
            if fact.event() == Some(&Event::AfterPlayerDeath(victim)) {
                vec![Box::new(Damage::new(None, avenger, 10))]
            } else {
                vec![]
            }
        },
    );
    world.run().expect("对局应正常结束");
    assert!(world.is_end());
    assert!(world.players().is_empty());
    assert_eq!(
        *trace.borrow(),
        vec![
            Event::DuelStart,
            Event::RoundStart { round: 1 },
            Event::BeforeTurn {
                round: 1,
                player_id: a
            },
            Event::Turn {
                round: 1,
                player_id: a
            },
            Event::BeforePlayerAttack {
                source_id: a,
                target_id: b
            },
            Event::PlayerAttack {
                source_id: a,
                target_id: b,
                damage: 10
            },
            Event::BeforePlayerDeath(b),
            Event::AfterPlayerDeath(b),
            Event::BeforePlayerDeath(a),
            Event::AfterPlayerDeath(a),
            Event::AfterPlayerAttack {
                source_id: a,
                target_id: b,
                damage: 10
            },
        ]
    );
    // 终局后的新根请求被无副作用拒绝。
    let result = world.execute(EmitEvent(Event::DuelStart));
    assert!(result.is_err(), "终局后应拒绝新的根请求");
    assert_eq!(trace.borrow().len(), 11);
}

#[test]
fn ordinary_attack_finishes_before_after_turn() {
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 1));
    let b = world.add_player(Player::new("B".into(), 10, 1));
    world.attach_component(Some(a), Abilities::new(a, vec![Box::new(Attack)]));
    let trace = trace_events(&mut world);
    world.run().expect("对局应正常结束");
    // Operation 路径的事件顺序是确定的：回合通知完成后才执行正常行动，
    // 行动及其全部反应完成后再发布 AfterTurn。
    assert_eq!(
        &trace.borrow()[3..8],
        &[
            Event::Turn {
                round: 1,
                player_id: a
            },
            Event::BeforePlayerAttack {
                source_id: a,
                target_id: b
            },
            Event::PlayerAttack {
                source_id: a,
                target_id: b,
                damage: 1
            },
            Event::AfterPlayerAttack {
                source_id: a,
                target_id: b,
                damage: 1
            },
            Event::AfterTurn {
                round: 1,
                player_id: a
            },
        ]
    );
    assert!(world.is_end(), "B 血量耗尽后对局应结束");
    assert!(world.player(b).is_none());
}
