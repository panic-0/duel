//! 回合、玩家行动、检查点与轮末结算。

use crate::support::{op_system, trace_events, Count};
use duel::core::{
    business::skills::Abilities,
    event::{Checkpoint, Event, EventKind},
    install_default_rules,
    operation::{AddPlayerOperation, EmitEvent, Operation, OperationError},
    player::Player,
    system::{EventEnvelope, Priority, ReactionTarget, System},
    Attack, BattleEngine, Damage, DuelRunner, Heal, PlayerId, RoundOperation,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// 记录所有 Turn 事件行动者的探针 System
#[derive(Debug)]
struct TurnSpy {
    turns: Rc<RefCell<Vec<PlayerId>>>,
}

impl System for TurnSpy {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::Turn, Priority::Final)]
    }

    fn candidates(
        &self,
        fact: &EventEnvelope<'_>,
        _query: &duel::core::query::Query<'_>,
    ) -> Vec<ReactionTarget> {
        matches!(fact.event(), Some(Event::Turn { .. }))
            .then(|| vec![ReactionTarget::Standalone])
            .unwrap_or_default()
    }

    fn respond(
        &self,
        fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        if let Some(Event::Turn { player_id, .. }) = fact.event() {
            self.turns.borrow_mut().push(*player_id);
        }
        Ok(vec![])
    }
}

#[test]
fn dead_player_never_gets_another_turn() {
    let turns = Rc::new(RefCell::new(Vec::new()));
    let mut world = BattleEngine::new();
    let a = world.add_player(Player::new("A".to_string(), 50, 5));
    let b = world.add_player(Player::new("B".to_string(), 5, 1));
    let c = world.add_player(Player::new("C".to_string(), 50, 5));
    world.register_system(TurnSpy {
        turns: turns.clone(),
    });
    for id in [a, b, c] {
        world.attach_component(Some(id), Abilities::new(id, vec![Box::new(Attack)]));
    }
    install_default_rules(&mut world);

    world.run().expect("对局应正常结束");

    assert!(world.is_end());
    let recorded = turns.borrow();
    assert!(!recorded.is_empty(), "探针应记录到真实的回合序列，而非空集");
    assert_eq!(&recorded[..2], &[a, c], "首轮依次是 A 与 C 的回合");
    assert_eq!(
        recorded.iter().filter(|&&id| id == b).count(),
        0,
        "B 在首次行动前已死亡，不应获得回合"
    );
}

/// 记录检查点阶段的探针。
#[derive(Debug)]
struct CheckpointRecorder(Rc<RefCell<Vec<Checkpoint>>>);

impl System for CheckpointRecorder {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::Checkpoint, Priority::Default)]
    }

    fn candidates(
        &self,
        fact: &EventEnvelope<'_>,
        _query: &duel::core::query::Query<'_>,
    ) -> Vec<ReactionTarget> {
        matches!(fact.event(), Some(Event::Checkpoint { .. }))
            .then(|| vec![ReactionTarget::Standalone])
            .unwrap_or_default()
    }

    fn respond(
        &self,
        fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
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
    let mut world = BattleEngine::new();
    world.add_player(Player::new("A".into(), 10, 0));
    world.add_player(Player::new("B".into(), 10, 0));
    let phases = Rc::new(RefCell::new(Vec::new()));
    world.register_system(CheckpointRecorder(phases.clone()));
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
    assert!(world.player(0).is_some());
    assert!(world.player(1).is_some());
}

#[test]
fn duel_operation_stops_at_its_own_round_limit() {
    let mut world = BattleEngine::new();
    world.add_player(Player::new("A".into(), 10, 0));
    world.add_player(Player::new("B".into(), 10, 0));
    world.run_with_max_rounds(1).expect("对局应正常结束");
    assert!(world.is_end());
    assert!(world.player(0).is_some());
    assert!(world.player(1).is_some());
}

#[derive(Debug)]
struct RoundEndCounter(Rc<Cell<usize>>);

impl System for RoundEndCounter {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![
            (EventKind::RoundEnd, Priority::Default),
            (EventKind::Checkpoint, Priority::Default),
        ]
    }
    fn candidates(
        &self,
        fact: &EventEnvelope<'_>,
        _query: &duel::core::query::Query<'_>,
    ) -> Vec<ReactionTarget> {
        match fact.event() {
            Some(Event::RoundEnd { .. }) => vec![ReactionTarget::Standalone],
            Some(Event::Checkpoint { phase, .. }) if *phase == Checkpoint::RoundEnd => {
                vec![ReactionTarget::Standalone]
            }
            _ => vec![],
        }
    }
    fn respond(
        &self,
        _fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(vec![Box::new(Count(self.0.clone()))])
    }
}

#[test]
fn a_standalone_round_finishes_its_own_end_event_and_checkpoint() {
    let mut world = BattleEngine::new();
    world.add_player(Player::new("A".into(), 10, 0));
    world.add_player(Player::new("B".into(), 10, 0));
    let count = Rc::new(Cell::new(0));
    world.register_system(RoundEndCounter(count.clone()));

    world
        .execute(RoundOperation { round: 1 })
        .expect("一个完整回合");

    assert_eq!(
        count.get(),
        2,
        "RoundOperation 应拥有轮末事件及检查点，而不要求调用者复制 DuelOperation 的收尾代码"
    );
}

#[test]
fn before_turn_death_skips_action_but_continues_round() {
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 1));
    let b = world.add_player(Player::new("B".into(), 10, 1));
    let c = world.add_player(Player::new("C".into(), 10, 1));
    let trace = trace_events(&mut world);
    op_system(&mut world, &[EventKind::BeforeTurn], move |fact, _| {
        match *fact.event().expect("事件事实") {
            Event::BeforeTurn { player_id, .. } if player_id == b => {
                // E 属于 BeforeTurn(B) 的结算批次，其死亡反应必须先完成。
                vec![Box::new(EmitEvent(Event::PlayerAttack {
                    source_id: a,
                    target_id: b,
                    damage: 10,
                })) as Box<dyn Operation>]
            }
            _ => vec![],
        }
    });
    op_system(
        &mut world,
        &[EventKind::PlayerAttack],
        |fact, _| match *fact.event().expect("事件事实") {
            Event::PlayerAttack {
                target_id, damage, ..
            } => vec![Box::new(Damage::new(None, target_id, damage)) as Box<dyn Operation>],
            _ => vec![],
        },
    );
    world.run_with_max_rounds(1).expect("对局应正常结束");
    let actors: Vec<_> = trace
        .borrow()
        .iter()
        .filter_map(|event| match event {
            Event::Turn { player_id, .. } => Some(*player_id),
            _ => None,
        })
        .collect();
    assert_eq!(actors, vec![a, c]);
    assert!(world.player(b).is_none());
    let trace = trace.borrow();
    let before_b = trace
        .iter()
        .position(|event| {
            *event
                == Event::BeforeTurn {
                    round: 1,
                    player_id: b,
                }
        })
        .unwrap();
    assert_eq!(
        &trace[before_b..before_b + 6],
        &[
            Event::BeforeTurn {
                round: 1,
                player_id: b
            },
            Event::PlayerAttack {
                source_id: a,
                target_id: b,
                damage: 10
            },
            Event::BeforePlayerDeath(b),
            Event::AfterPlayerDeath(b),
            Event::BeforeTurn {
                round: 1,
                player_id: c
            },
            Event::Turn {
                round: 1,
                player_id: c
            },
        ]
    );
}

#[test]
fn actor_dying_during_its_turn_still_gets_after_turn_when_game_continues() {
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 1));
    world.add_player(Player::new("B".into(), 10, 1));
    world.add_player(Player::new("C".into(), 10, 1));
    let after_turns = Rc::new(RefCell::new(Vec::new()));
    let sink = after_turns.clone();
    op_system(&mut world, &[EventKind::Turn], move |fact, _| {
        match *fact.event().expect("事件事实") {
            Event::Turn { player_id, .. } if player_id == a => {
                vec![Box::new(Damage::new(None, a, 10)) as Box<dyn Operation>]
            }
            _ => vec![],
        }
    });
    op_system(&mut world, &[EventKind::AfterTurn], move |fact, _| {
        if let Some(Event::AfterTurn { player_id, .. }) = fact.event() {
            sink.borrow_mut().push(*player_id);
        }
        vec![]
    });
    world.run_with_max_rounds(1).expect("对局应正常结束");
    assert_eq!(*after_turns.borrow(), vec![0, 1, 2]);
}

#[test]
fn zero_round_limit_finishes_start_effects_without_starting_a_round() {
    let mut world = BattleEngine::new();
    world.add_player(Player::new("A".into(), 10, 1));
    world.add_player(Player::new("B".into(), 10, 1));
    let trace = trace_events(&mut world);
    op_system(&mut world, &[EventKind::DuelStart], |_, _| {
        vec![
            Box::new(AddPlayerOperation(Player::new("开局召唤物".into(), 10, 1)))
                as Box<dyn Operation>,
        ]
    });
    world.run_with_max_rounds(0).expect("对局应正常结束");
    assert!(world.is_end());
    assert_eq!(world.players().len(), 3);
    assert_eq!(*trace.borrow(), vec![Event::DuelStart]);
}

#[test]
fn round_limit_ends_after_round_end_reactions_without_needing_another_round() {
    let mut world = BattleEngine::new();
    let a = world.add_player(Player::new("A".into(), 10, 1));
    world.add_player(Player::new("B".into(), 10, 1));
    assert!(world.set_initial_hp(a, 5));
    op_system(&mut world, &[EventKind::RoundEnd], move |_, _| {
        vec![Box::new(Heal::new(a, 2)) as Box<dyn Operation>]
    });
    world.run_with_max_rounds(1).expect("对局应正常结束");
    assert_eq!(world.player(a).unwrap().hp(), 7, "回合结束的反应应先完成");
    assert!(world.is_end(), "无需等待下一回合事件就应结束");
}
