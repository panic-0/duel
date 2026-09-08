use std::{cell::RefCell, fmt, rc::Rc};

use duel::core::{
    ability::{Abilities, Attack},
    buff::{Buff, Priority},
    command::{AddPlayer, ApplyEvent, Command, Commands},
    event::{Event, EventType},
    flow::{EndCondition, FlowDriver, GameResult},
    log::{LogEntry, Logger},
    modifier::HpModifier,
    operation::{AddPlayerOperation, Damage, EmitEvent, Heal, Operation},
    player::Player,
    state::GameState,
    world::World,
    BuffId,
};

type Handler = dyn FnMut(&mut Event, &GameState, &mut Commands, BuffId);

struct Effect {
    subscriptions: Vec<(EventType, Priority)>,
    handler: Box<Handler>,
}

impl fmt::Debug for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Effect")
    }
}

impl Buff for Effect {
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        self.subscriptions.clone()
    }

    fn on_event(
        &mut self,
        event: &mut Event,
        state: &GameState,
        commands: &mut Commands,
        id: BuffId,
    ) {
        (self.handler)(event, state, commands, id);
    }
}

fn effect(
    world: &mut World,
    events: &[EventType],
    handler: impl FnMut(&mut Event, &GameState, &mut Commands, BuffId) + 'static,
) {
    world.add_buff(Box::new(Effect {
        subscriptions: events
            .iter()
            .map(|&event| (event, Priority::Default))
            .collect(),
        handler: Box::new(handler),
    }));
}

/// Operation 路径的监听者：以不可变事件产生后续操作。
type OpHandler = dyn FnMut(&Event, &GameState) -> Vec<Box<dyn Operation>>;

struct OpEffect {
    subscriptions: Vec<(EventType, Priority)>,
    handler: Box<OpHandler>,
}

impl fmt::Debug for OpEffect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OpEffect")
    }
}

impl Buff for OpEffect {
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        self.subscriptions.clone()
    }

    fn operations(
        &mut self,
        event: &Event,
        state: &GameState,
        _buff_id: BuffId,
    ) -> Vec<Box<dyn Operation>> {
        (self.handler)(event, state)
    }
}

fn op_effect(
    world: &mut World,
    events: &[EventType],
    handler: impl FnMut(&Event, &GameState) -> Vec<Box<dyn Operation>> + 'static,
) {
    world.add_buff(Box::new(OpEffect {
        subscriptions: events
            .iter()
            .map(|&event| (event, Priority::Default))
            .collect(),
        handler: Box::new(handler),
    }));
}

#[derive(Debug)]
struct NoFlow;

impl FlowDriver for NoFlow {
    fn advance(&mut self, _: &GameState, _: &Event) -> Vec<Event> {
        vec![]
    }
}

#[test]
fn apply_event_settles_depth_first_without_consuming_external_events() {
    let mut world = World::new();
    world.set_flow(Box::new(NoFlow));
    let trace = Rc::new(RefCell::new(Vec::new()));
    let sink = trace.clone();
    effect(
        &mut world,
        &[EventType::RoundStart, EventType::RoundEnd],
        move |event, _, commands, _| {
            sink.borrow_mut().push(event.clone());
            let children = match event {
                Event::RoundStart { round: 1 } => {
                    vec![Event::RoundStart { round: 2 }, Event::RoundEnd { round: 1 }]
                }
                Event::RoundStart { round: 2 } => vec![Event::RoundStart { round: 3 }],
                _ => vec![],
            };
            for event in children {
                commands.push(ApplyEvent { event });
            }
        },
    );
    world.queue_event(Event::RoundStart { round: 99 });
    world.apply_event(&mut Event::RoundStart { round: 1 });
    assert_eq!(
        *trace.borrow(),
        vec![
            Event::RoundStart { round: 1 },
            Event::RoundStart { round: 2 },
            Event::RoundStart { round: 3 },
            Event::RoundEnd { round: 1 },
        ]
    );
    world.pump();
    assert_eq!(
        trace.borrow().last(),
        Some(&Event::RoundStart { round: 99 })
    );
}

#[test]
fn death_confirmation_waits_for_queued_revival() {
    let mut world = World::new();
    world.set_flow(Box::new(NoFlow));
    let hero = world.add_player(Player::new("英雄".into(), 10, 1));
    let deaths = Rc::new(RefCell::new(0));
    let sink = deaths.clone();
    effect(
        &mut world,
        &[
            EventType::BeforePlayerDeath,
            EventType::RoundEnd,
            EventType::AfterPlayerDeath,
        ],
        move |event, _, commands, _| match event {
            Event::BeforePlayerDeath(_) => commands.push(ApplyEvent {
                event: Event::RoundEnd { round: 1 },
            }),
            Event::RoundEnd { .. } => commands.push(HpModifier::heal(hero, 5)),
            Event::AfterPlayerDeath(_) => *sink.borrow_mut() += 1,
            _ => {}
        },
    );
    Box::new(HpModifier::damage(hero, 10)).apply(&mut world);
    world.pump();
    assert_eq!(world.get_player(hero).map(|p| p.hp()), Some(5));
    assert_eq!(*deaths.borrow(), 0);
}

#[test]
fn death_summon_finishes_before_last_survivor_check() {
    let mut world = World::new();
    world.add_player(Player::new("攻击者".into(), 10, 10));
    let target = world.add_player(Player::new("召唤者".into(), 10, 1));
    world.add_buff(Box::new(Abilities::new(0, vec![Box::new(Attack)])));
    op_effect(&mut world, &[EventType::AfterPlayerDeath], |_, _| {
        vec![
            Box::new(AddPlayerOperation(Player::new("召唤物".into(), 10, 1))) as Box<dyn Operation>,
        ]
    });
    // 召唤发生在死亡通知内、下一个检查点之前，
    // 因此“只剩一人”的胜负判断始终不成立，对局一直有新对手。
    world.run_with_max_rounds(4).expect("对局应正常结束");
    assert!(world.get_player(target).is_none());
    assert_eq!(
        world.get_players().len(),
        2,
        "召唤物应接替死亡者，使对局持续"
    );
    assert!(world.is_end(), "到达回合上限后以平局结束");
}

#[test]
fn stale_death_events_do_not_reach_listeners() {
    let mut world = World::new();
    world.set_flow(Box::new(NoFlow));
    let hero = world.add_player(Player::new("英雄".into(), 10, 1));
    let calls = Rc::new(RefCell::new(0));
    let sink = calls.clone();
    effect(
        &mut world,
        &[EventType::BeforePlayerDeath, EventType::AfterPlayerDeath],
        move |_, _, _, _| *sink.borrow_mut() += 1,
    );
    world.apply_event(&mut Event::BeforePlayerDeath(hero));
    world.remove_player(hero);
    world.apply_event(&mut Event::BeforePlayerDeath(hero));
    assert_eq!(*calls.borrow(), 0);
}

#[test]
fn actor_dying_during_its_turn_still_gets_after_turn_when_game_continues() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 1));
    world.add_player(Player::new("B".into(), 10, 1));
    world.add_player(Player::new("C".into(), 10, 1));
    let after_turns = Rc::new(RefCell::new(Vec::new()));
    let sink = after_turns.clone();
    op_effect(
        &mut world,
        &[EventType::Turn],
        move |event, _| match *event {
            Event::Turn { player_id, .. } if player_id == a => {
                vec![Box::new(Damage::new(None, a, 10)) as Box<dyn Operation>]
            }
            _ => vec![],
        },
    );
    op_effect(&mut world, &[EventType::AfterTurn], move |event, _| {
        if let Event::AfterTurn { player_id, .. } = *event {
            sink.borrow_mut().push(player_id);
        }
        vec![]
    });
    world.run_with_max_rounds(1).expect("对局应正常结束");
    assert_eq!(*after_turns.borrow(), vec![0, 1, 2]);
}

fn trace_events(world: &mut World) -> Rc<RefCell<Vec<Event>>> {
    let trace = Rc::new(RefCell::new(Vec::new()));
    let sink = trace.clone();
    op_effect(
        world,
        &[
            EventType::DuelStart,
            EventType::RoundStart,
            EventType::BeforeTurn,
            EventType::Turn,
            EventType::AfterTurn,
            EventType::RoundEnd,
            EventType::BeforePlayerAttack,
            EventType::PlayerAttack,
            EventType::AfterPlayerAttack,
            EventType::BeforePlayerDeath,
            EventType::AfterPlayerDeath,
        ],
        move |event, _state| {
            sink.borrow_mut().push(event.clone());
            vec![]
        },
    );
    trace
}

#[test]
fn fatal_attack_completes_notifications_and_retaliation_before_ending() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 10));
    let b = world.add_player(Player::new("B".into(), 10, 1));
    world.add_buff(Box::new(Abilities::new(a, vec![Box::new(Attack)])));
    let trace = trace_events(&mut world);
    let victim = b;
    let avenger = a;
    op_effect(
        &mut world,
        &[EventType::AfterPlayerDeath],
        move |event, _state| {
            if *event == Event::AfterPlayerDeath(victim) {
                vec![Box::new(Damage::new(None, avenger, 10))]
            } else {
                vec![]
            }
        },
    );
    world.run().expect("对局应正常结束");
    assert!(world.is_end());
    assert!(world.get_players().is_empty());
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
    // 终局后再次驱动不会重发通知。
    world.pump();
    world.apply_event(&mut Event::DuelStart);
    assert_eq!(trace.borrow().len(), 11);
}

#[test]
fn ordinary_attack_finishes_before_after_turn() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 1));
    let b = world.add_player(Player::new("B".into(), 10, 1));
    world.add_buff(Box::new(Abilities::new(a, vec![Box::new(Attack)])));
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
    assert!(world.get_player(b).is_none());
}

#[test]
fn before_turn_death_skips_action_but_continues_round() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 1));
    let b = world.add_player(Player::new("B".into(), 10, 1));
    let c = world.add_player(Player::new("C".into(), 10, 1));
    let trace = trace_events(&mut world);
    op_effect(&mut world, &[EventType::BeforeTurn], move |event, _| {
        match *event {
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
    op_effect(
        &mut world,
        &[EventType::PlayerAttack],
        |event, _| match *event {
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
    assert!(world.get_player(b).is_none());
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
fn duplicate_death_reports_emit_one_notification_and_log() {
    let mut world = World::new();
    world.set_flow(Box::new(NoFlow));
    let a = world.add_player(Player::new("A".into(), 10, 1));
    let trace = trace_events(&mut world);
    let logs = Rc::new(RefCell::new(Vec::new()));
    let sink = logs.clone();
    world.set_logger(Logger::new(Box::new(move |_, entry| {
        sink.borrow_mut().push(entry.clone())
    })));
    effect(
        &mut world,
        &[EventType::RoundEnd],
        move |_, _, commands, _| {
            commands.push(HpModifier::damage(a, 10));
            commands.push(ApplyEvent {
                event: Event::BeforePlayerDeath(a),
            });
        },
    );
    world.apply_event(&mut Event::RoundEnd { round: 1 });
    assert_eq!(
        trace
            .borrow()
            .iter()
            .filter(|e| **e == Event::BeforePlayerDeath(a))
            .count(),
        1
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
        1
    );
}

#[derive(Debug)]
struct StateSpyFlow(Rc<RefCell<Vec<(Event, u64)>>>);

impl FlowDriver for StateSpyFlow {
    fn advance(&mut self, state: &GameState, event: &Event) -> Vec<Event> {
        self.0
            .borrow_mut()
            .push((event.clone(), state.get_player(0).unwrap().hp()));
        if *event == (Event::RoundEnd { round: 1 }) {
            vec![Event::RoundEnd { round: 9 }]
        } else {
            vec![]
        }
    }
}

#[test]
fn flow_observes_mutated_events_in_dispatch_order_with_final_state() {
    let mut world = World::new();
    world.add_player(Player::new("A".into(), 10, 1));
    let calls = Rc::new(RefCell::new(Vec::new()));
    world.set_flow(Box::new(StateSpyFlow(calls.clone())));
    effect(
        &mut world,
        &[EventType::PlayerAttack, EventType::RoundEnd],
        |event, _, commands, _| match event {
            Event::PlayerAttack { damage, .. } => {
                *damage = 3;
                commands.push(ApplyEvent {
                    event: Event::RoundEnd { round: 1 },
                });
            }
            Event::RoundEnd { round: 1 } => commands.push(HpModifier::damage(0, 3)),
            _ => {}
        },
    );
    let mut event = Event::PlayerAttack {
        source_id: 0,
        target_id: 0,
        damage: 10,
    };
    world.apply_event(&mut event);
    assert_eq!(
        event,
        Event::PlayerAttack {
            source_id: 0,
            target_id: 0,
            damage: 3
        }
    );
    assert_eq!(
        *calls.borrow(),
        vec![
            (event, 7),
            (Event::RoundEnd { round: 1 }, 7),
            (
                Event::HpChanged {
                    target_id: 0,
                    old_hp: 10,
                    new_hp: 7
                },
                7
            ),
        ]
    );
    world.pump();
    assert_eq!(
        calls.borrow().last(),
        Some(&(Event::RoundEnd { round: 9 }, 7))
    );
}

#[derive(Debug)]
struct ConditionSpy(Rc<RefCell<Vec<(Event, usize)>>>);

impl EndCondition for ConditionSpy {
    fn check(&mut self, state: &GameState, event: &Event) -> Option<GameResult> {
        self.0
            .borrow_mut()
            .push((event.clone(), state.get_players().len()));
        None
    }
}

#[test]
fn end_conditions_check_root_once_after_settlement() {
    let mut world = World::new();
    world.set_flow(Box::new(NoFlow));
    let calls = Rc::new(RefCell::new(Vec::new()));
    world.add_end_condition(Box::new(ConditionSpy(calls.clone())));
    effect(
        &mut world,
        &[EventType::RoundStart, EventType::RoundEnd],
        |event, _, commands, _| match event {
            Event::RoundStart { .. } => commands.push(ApplyEvent {
                event: Event::RoundEnd { round: 1 },
            }),
            Event::RoundEnd { .. } => commands.push(AddPlayer {
                player: Player::new("A".into(), 10, 1),
            }),
            _ => {}
        },
    );
    world.apply_event(&mut Event::RoundStart { round: 1 });
    assert_eq!(*calls.borrow(), vec![(Event::RoundStart { round: 1 }, 1)]);
}

#[test]
fn round_limit_ends_after_round_end_reactions_without_needing_another_round() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 1));
    world.add_player(Player::new("B".into(), 10, 1));
    world.get_player_mut(a).unwrap().set_hp(5);
    op_effect(&mut world, &[EventType::RoundEnd], move |_, _| {
        vec![Box::new(Heal::new(a, 2)) as Box<dyn Operation>]
    });
    world.run_with_max_rounds(1).expect("对局应正常结束");
    assert_eq!(
        world.get_player(a).unwrap().hp(),
        7,
        "回合结束的反应应先完成"
    );
    assert!(world.is_end(), "无需等待下一回合事件就应结束");
}

#[test]
fn zero_round_limit_finishes_start_effects_without_starting_a_round() {
    let mut world = World::new();
    world.add_player(Player::new("A".into(), 10, 1));
    world.add_player(Player::new("B".into(), 10, 1));
    let trace = trace_events(&mut world);
    op_effect(&mut world, &[EventType::DuelStart], |_, _| {
        vec![
            Box::new(AddPlayerOperation(Player::new("开局召唤物".into(), 10, 1)))
                as Box<dyn Operation>,
        ]
    });
    world.run_with_max_rounds(0).expect("对局应正常结束");
    assert!(world.is_end());
    assert_eq!(world.get_players().len(), 3);
    assert_eq!(*trace.borrow(), vec![Event::DuelStart]);
}

#[test]
fn long_reaction_chain_completes_iteratively() {
    let mut world = World::new();
    world.set_flow(Box::new(NoFlow));
    let count = Rc::new(RefCell::new(0));
    let sink = count.clone();
    effect(
        &mut world,
        &[EventType::RoundEnd],
        move |event, _, commands, _| {
            if let Event::RoundEnd { round } = event {
                *sink.borrow_mut() += 1;
                if *round < 20_000 {
                    commands.push(ApplyEvent {
                        event: Event::RoundEnd { round: *round + 1 },
                    });
                }
            }
        },
    );
    world.apply_event(&mut Event::RoundEnd { round: 1 });
    assert_eq!(*count.borrow(), 20_000);
}

#[derive(Debug, Clone, Copy)]
enum Reenter {
    Apply,
    Pump,
    Run,
}

impl Command for Reenter {
    fn apply(self: Box<Self>, world: &mut World) {
        match *self {
            Self::Apply => world.apply_event(&mut Event::RoundEnd { round: 1 }),
            Self::Pump => world.pump(),
            Self::Run => {
                let _ = world.run();
            }
        }
    }
}

#[test]
fn driver_reentry_is_rejected_for_all_entry_points() {
    for entry in [Reenter::Apply, Reenter::Pump, Reenter::Run] {
        let mut world = World::new();
        effect(
            &mut world,
            &[EventType::RoundEnd],
            move |_, _, commands, _| commands.push(entry),
        );
        let error = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            world.apply_event(&mut Event::RoundEnd { round: 1 })
        }))
        .unwrap_err();
        let message = error
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| error.downcast_ref::<String>().map(String::as_str))
            .unwrap();
        assert!(message.contains("不能在结算期间重入驱动"));
    }
}
