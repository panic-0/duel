//! 结算行为测试：深度优先、死亡确认窗口、终局检查时机与攻击通知完整性。
//! 全部走统一执行路径：Operation → 同步通知 → System 响应 → 子 Operation。

use std::{cell::RefCell, fmt, rc::Rc};

use duel::core::{
    business::skills::Abilities,
    event::{Event, EventType},
    install_default_rules,
    log::{LogEntry, Logger},
    operation::{AddPlayerOperation, EmitEvent, Operation, OperationError},
    player::Player,
    state::GameState,
    system::{Fact, NoticeKind, Priority, Subject, System},
    Attack, Damage, DuelRunner, Heal, World,
};

type Handler = dyn Fn(&Fact<'_>, &GameState) -> Vec<Box<dyn Operation>>;

/// 通用响应 System：候选为独立响应，handler 决定提出的操作。
struct OpSystem {
    subscriptions: Vec<(NoticeKind, Priority)>,
    handler: Box<Handler>,
}

impl fmt::Debug for OpSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OpSystem")
    }
}

impl System for OpSystem {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        self.subscriptions.clone()
    }

    fn candidates(&self, _fact: &Fact<'_>, _query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        vec![Subject::Standalone]
    }

    fn respond(
        &self,
        fact: &Fact<'_>,
        _subject: Subject,
        query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok((self.handler)(fact, query.state()))
    }
}

fn op_system(
    world: &mut World,
    events: &[EventType],
    handler: impl Fn(&Fact<'_>, &GameState) -> Vec<Box<dyn Operation>> + 'static,
) {
    world.add_system(OpSystem {
        subscriptions: events
            .iter()
            .map(|&event| (NoticeKind::Event(event), Priority::Default))
            .collect(),
        handler: Box::new(handler),
    });
}

fn trace_events(world: &mut World) -> Rc<RefCell<Vec<Event>>> {
    let trace = Rc::new(RefCell::new(Vec::new()));
    let sink = trace.clone();
    op_system(
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
        move |fact, _state| {
            if let Some(event) = fact.event() {
                sink.borrow_mut().push(event.clone());
            }
            vec![]
        },
    );
    trace
}

#[test]
fn fatal_attack_completes_notifications_and_retaliation_before_ending() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 10));
    let b = world.add_player(Player::new("B".into(), 10, 1));
    world.add_data(Some(a), Abilities::new(vec![Box::new(Attack)]));
    let trace = trace_events(&mut world);
    let victim = b;
    let avenger = a;
    op_system(
        &mut world,
        &[EventType::AfterPlayerDeath],
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
    // 终局后的新根请求被无副作用拒绝。
    let result = world.execute(EmitEvent(Event::DuelStart));
    assert!(result.is_err(), "终局后应拒绝新的根请求");
    assert_eq!(trace.borrow().len(), 11);
}

#[test]
fn ordinary_attack_finishes_before_after_turn() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 1));
    let b = world.add_player(Player::new("B".into(), 10, 1));
    world.add_data(Some(a), Abilities::new(vec![Box::new(Attack)]));
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
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 1));
    let b = world.add_player(Player::new("B".into(), 10, 1));
    let c = world.add_player(Player::new("C".into(), 10, 1));
    let trace = trace_events(&mut world);
    op_system(&mut world, &[EventType::BeforeTurn], move |fact, _| {
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
        &[EventType::PlayerAttack],
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
fn actor_dying_during_its_turn_still_gets_after_turn_when_game_continues() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let a = world.add_player(Player::new("A".into(), 10, 1));
    world.add_player(Player::new("B".into(), 10, 1));
    world.add_player(Player::new("C".into(), 10, 1));
    let after_turns = Rc::new(RefCell::new(Vec::new()));
    let sink = after_turns.clone();
    op_system(&mut world, &[EventType::Turn], move |fact, _| {
        match *fact.event().expect("事件事实") {
            Event::Turn { player_id, .. } if player_id == a => {
                vec![Box::new(Damage::new(None, a, 10)) as Box<dyn Operation>]
            }
            _ => vec![],
        }
    });
    op_system(&mut world, &[EventType::AfterTurn], move |fact, _| {
        if let Some(Event::AfterTurn { player_id, .. }) = fact.event() {
            sink.borrow_mut().push(*player_id);
        }
        vec![]
    });
    world.run_with_max_rounds(1).expect("对局应正常结束");
    assert_eq!(*after_turns.borrow(), vec![0, 1, 2]);
}

#[test]
fn death_summon_finishes_before_last_survivor_check() {
    let mut world = World::new();
    install_default_rules(&mut world);
    world.add_player(Player::new("攻击者".into(), 10, 10));
    let target = world.add_player(Player::new("召唤者".into(), 10, 1));
    world.add_data(Some(0), Abilities::new(vec![Box::new(Attack)]));
    op_system(&mut world, &[EventType::AfterPlayerDeath], |_, _| {
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
fn zero_round_limit_finishes_start_effects_without_starting_a_round() {
    let mut world = World::new();
    world.add_player(Player::new("A".into(), 10, 1));
    world.add_player(Player::new("B".into(), 10, 1));
    let trace = trace_events(&mut world);
    op_system(&mut world, &[EventType::DuelStart], |_, _| {
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
fn round_limit_ends_after_round_end_reactions_without_needing_another_round() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 1));
    world.add_player(Player::new("B".into(), 10, 1));
    world.get_player_mut(a).unwrap().set_hp(5);
    op_system(&mut world, &[EventType::RoundEnd], move |_, _| {
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
fn duplicate_death_requests_emit_one_notification_and_log() {
    let mut world = World::new();
    let a = world.add_player(Player::new("A".into(), 10, 1));
    let trace = trace_events(&mut world);
    let logs = Rc::new(RefCell::new(Vec::new()));
    let sink = logs.clone();
    world.set_logger(Logger::new(Box::new(move |_, entry| {
        sink.borrow_mut().push(entry.clone())
    })));
    // 同一响应提出两次死亡请求：防重入应让第二次跳过。
    op_system(&mut world, &[EventType::HpChanged], move |fact, state| {
        if !matches!(fact.event(), Some(Event::HpChanged { .. })) {
            return vec![];
        }
        state
            .get_players()
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
    assert!(world.get_player(a).is_none());
}

#[test]
fn rescue_during_before_death_prevents_death_notifications() {
    let mut world = World::new();
    install_default_rules(&mut world);
    let hero = world.add_player(Player::new("英雄".into(), 10, 1));
    let deaths = Rc::new(RefCell::new(0));
    let sink = deaths.clone();
    op_system(&mut world, &[EventType::BeforePlayerDeath], move |_, _| {
        vec![Box::new(Heal::new(hero, 5)) as Box<dyn Operation>]
    });
    op_system(&mut world, &[EventType::AfterPlayerDeath], move |_, _| {
        *sink.borrow_mut() += 1;
        vec![]
    });
    world
        .execute(Damage::new(None, hero, 10))
        .expect("致命伤害应正常结算");
    assert_eq!(world.get_player(hero).map(|p| p.hp()), Some(5));
    assert_eq!(*deaths.borrow(), 0, "救回成功后不应有死亡后通知");
}

#[test]
fn event_reactions_settle_depth_first_before_their_siblings() {
    let mut world = World::new();
    let trace = Rc::new(RefCell::new(Vec::new()));
    let sink = trace.clone();
    op_system(&mut world, &[EventType::RoundStart, EventType::RoundEnd], {
        let sink = sink.clone();
        move |fact, _| {
            match fact.event() {
                Some(Event::RoundStart { round }) => sink.borrow_mut().push(*round),
                Some(Event::RoundEnd { .. }) => sink.borrow_mut().push(0),
                _ => {}
            }
            let Some(Event::RoundStart { round }) = fact.event() else {
                return vec![];
            };
            let children = match *round {
                1 => vec![2, 0],
                2 => vec![3],
                _ => vec![],
            };
            children
                .into_iter()
                .map(|round| {
                    let op: Box<dyn Operation> = if round == 0 {
                        Box::new(EmitEvent(Event::RoundEnd { round: 1 }))
                    } else {
                        Box::new(EmitEvent(Event::RoundStart { round }))
                    };
                    op
                })
                .collect()
        }
    });
    world
        .execute(EmitEvent(Event::RoundStart { round: 1 }))
        .expect("事件应正常分发");
    // 子通知先于兄弟通知完整结算：1 → 2 → 3 → 回合结束。
    assert_eq!(*trace.borrow(), vec![1, 2, 3, 0]);
}

#[test]
fn long_event_reaction_chain_settles_within_depth_budget() {
    let mut world = World::new();
    let count = Rc::new(RefCell::new(0));
    let sink = count.clone();
    op_system(&mut world, &[EventType::RoundEnd], move |fact, _| {
        if let Some(Event::RoundEnd { round }) = fact.event() {
            *sink.borrow_mut() += 1;
            if *round < 150 {
                return vec![Box::new(EmitEvent(Event::RoundEnd { round: *round + 1 }))];
            }
        }
        vec![]
    });
    world
        .execute(EmitEvent(Event::RoundEnd { round: 1 }))
        .expect("链长在深度预算内应正常结算");
    assert_eq!(*count.borrow(), 150);
}
