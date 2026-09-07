use std::cell::RefCell;
use std::rc::Rc;

use duel::core::{
    ability::{Abilities, Attack},
    buff::{Buff, DamageReduction, Priority, Revival},
    command::Commands,
    event::{Event, EventType},
    flow::{FlowDriver, RoundLimit},
    log::{LogEntry, Logger},
    player::Player,
    state::GameState,
    world::World,
    BuffId, PlayerId,
};

/// 记录自己被触发顺序的假 buff，用于测试事件分发
#[derive(Debug)]
struct RecordBuff {
    tag: u8,
    event_type: EventType,
    priority: Priority,
    triggered: Rc<RefCell<Vec<u8>>>,
}

impl RecordBuff {
    fn new(
        tag: u8,
        event_type: EventType,
        priority: Priority,
        triggered: Rc<RefCell<Vec<u8>>>,
    ) -> Self {
        RecordBuff {
            tag,
            event_type,
            priority,
            triggered,
        }
    }
}

impl Buff for RecordBuff {
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        vec![(self.event_type, self.priority)]
    }

    fn on_event(
        &mut self,
        _event: &mut Event,
        _world: &GameState,
        _commands: &mut Commands,
        _buff_id: BuffId,
    ) {
        self.triggered.borrow_mut().push(self.tag);
    }
}

fn log_sink() -> (Rc<RefCell<Vec<LogEntry>>>, Logger) {
    let entries = Rc::new(RefCell::new(Vec::new()));
    let sink = entries.clone();
    let logger = Logger::new(Box::new(move |_world, entry| {
        sink.borrow_mut().push(entry.clone());
    }));
    (entries, logger)
}

fn count(entries: &[LogEntry], pred: impl Fn(&LogEntry) -> bool) -> usize {
    entries.iter().filter(|e| pred(e)).count()
}

#[test]
fn full_game_runs_to_expected_outcome() {
    let mut world = World::new();
    let p1 = world.add_player(Player::new("Player1".to_string(), 15, 10));
    let p2 = world.add_player(Player::new("Player2".to_string(), 28, 8));
    world.add_buff(Box::new(Abilities::new(p1, vec![Box::new(Attack)])));
    world.add_buff(Box::new(Revival::new(p1)));
    world.add_buff(Box::new(Abilities::new(p2, vec![Box::new(Attack)])));
    world.add_buff(Box::new(DamageReduction::new(p2, 0.2)));

    world.run();

    assert!(world.is_end());
    assert!(world.get_player(p1).is_none(), "Player1 应已死亡移除");
    let winner = world.get_player(p2).unwrap();
    assert_eq!(winner.hp(), 4);
    assert!(winner.is_alive());
}

#[test]
fn revival_triggers_exactly_once() {
    let (entries, logger) = log_sink();
    let mut world = World::new();
    world.set_logger(logger);

    let p1 = world.add_player(Player::new("Hero".to_string(), 10, 2));
    let p2 = world.add_player(Player::new("Boss".to_string(), 100, 10));
    world.add_buff(Box::new(Abilities::new(p1, vec![Box::new(Attack)])));
    world.add_buff(Box::new(Revival::new(p1)));
    world.add_buff(Box::new(Abilities::new(p2, vec![Box::new(Attack)])));

    world.run();

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
    assert!(world.get_player(p1).is_none());
    assert_eq!(world.get_player(p2).unwrap().hp(), 96);
}

#[test]
fn higher_priority_listeners_trigger_first() {
    let triggered = Rc::new(RefCell::new(Vec::new()));
    let mut world = World::new();
    world.add_buff(Box::new(RecordBuff::new(
        1,
        EventType::PlayerAttack,
        Priority::Default,
        triggered.clone(),
    )));
    world.add_buff(Box::new(RecordBuff::new(
        2,
        EventType::PlayerAttack,
        Priority::Modify,
        triggered.clone(),
    )));
    world.add_buff(Box::new(RecordBuff::new(
        3,
        EventType::PlayerAttack,
        Priority::Default,
        triggered.clone(),
    )));

    assert_eq!(
        world.get_event_registration_count(EventType::PlayerAttack),
        3
    );
    assert!(world.validate_registry_consistency());
    assert_eq!(world.get_registry_stats(), (1, 3, 3));

    world.apply_event(&mut Event::PlayerAttack {
        source_id: 0,
        target_id: 1,
        damage: 5,
    });

    assert_eq!(*triggered.borrow(), vec![2, 1, 3]);
}

#[test]
fn removed_buff_no_longer_receives_events() {
    let triggered = Rc::new(RefCell::new(Vec::new()));
    let mut world = World::new();
    let b1 = world.add_buff(Box::new(RecordBuff::new(
        1,
        EventType::PlayerAttack,
        Priority::Default,
        triggered.clone(),
    )));
    world.add_buff(Box::new(RecordBuff::new(
        2,
        EventType::PlayerAttack,
        Priority::Default,
        triggered.clone(),
    )));

    world.remove_buff(b1);
    assert!(world.validate_registry_consistency());
    assert_eq!(
        world.get_event_registration_count(EventType::PlayerAttack),
        1
    );

    world.apply_event(&mut Event::PlayerAttack {
        source_id: 0,
        target_id: 1,
        damage: 5,
    });
    assert_eq!(*triggered.borrow(), vec![2]);
}

#[test]
fn round_limit_stops_unwinnable_game() {
    let (entries, logger) = log_sink();
    let mut world = World::new();
    world.set_logger(logger);

    let p1 = world.add_player(Player::new("A".to_string(), 10, 2));
    let p2 = world.add_player(Player::new("B".to_string(), 10, 2));
    world.add_buff(Box::new(Abilities::new(p1, vec![Box::new(Attack)])));
    world.add_buff(Box::new(DamageReduction::new(p1, 1.0)));
    world.add_buff(Box::new(Abilities::new(p2, vec![Box::new(Attack)])));
    world.add_buff(Box::new(DamageReduction::new(p2, 1.0)));

    world.run();

    assert!(world.is_end(), "无限对局应被回合上限终止");
    assert!(world.get_player(p1).is_some(), "双方应都存活");
    assert!(world.get_player(p2).is_some());
    assert!(entries.borrow().contains(&LogEntry::Draw), "应记录平局日志");
}

// —— 可插拔流程与结束条件 ——

/// 不生成任何后续事件的流程驱动
#[derive(Debug)]
struct NoFlow;

impl FlowDriver for NoFlow {
    fn advance(&mut self, _state: &GameState, _event: &Event) -> Vec<Event> {
        vec![]
    }
}

#[test]
fn custom_flow_replaces_round_structure() {
    let mut world = World::new();
    world.add_player(Player::new("A".to_string(), 10, 5));
    world.add_player(Player::new("B".to_string(), 10, 5));
    world.set_flow(Box::new(NoFlow));

    world.run();

    // 没有任何流程事件生成，对局停留在 DuelStart 之后，不判结束
    assert!(!world.is_end());
    assert!(world.get_player(0).is_some());
    assert!(world.get_player(1).is_some());
}

#[test]
fn three_player_game_ends_with_single_survivor() {
    let mut world = World::new();
    for name in ["A", "B", "C"] {
        world.add_player(Player::new(name.to_string(), 30, 10));
    }
    for id in 0..3 {
        world.add_buff(Box::new(Abilities::new(id, vec![Box::new(Attack)])));
    }

    world.run();

    assert!(world.is_end(), "三人对局应打到只剩一人");
    let survivors: Vec<_> = world.get_players().keys().copied().collect();
    assert_eq!(survivors.len(), 1);
}

#[test]
fn custom_round_limit_ends_game_early() {
    let mut world = World::new();
    world.add_player(Player::new("A".to_string(), 100, 5));
    world.add_player(Player::new("B".to_string(), 100, 5));
    world.add_buff(Box::new(Abilities::new(0, vec![Box::new(Attack)])));
    world.add_buff(Box::new(Abilities::new(1, vec![Box::new(Attack)])));
    world.add_end_condition(Box::new(RoundLimit::new(2)));

    world.run();

    assert!(world.is_end(), "应因自定义回合上限提前结束");
    assert!(world.get_player(0).is_some(), "2 回合内双方都应存活");
    assert!(world.get_player(1).is_some());
}

/// 记录所有 Turn 事件行动者的探针 buff
#[derive(Debug)]
struct TurnSpy {
    turns: Rc<RefCell<Vec<PlayerId>>>,
}

impl Buff for TurnSpy {
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        vec![(EventType::Turn, Priority::Final)]
    }

    fn on_event(
        &mut self,
        event: &mut Event,
        _world: &GameState,
        _commands: &mut Commands,
        _buff_id: BuffId,
    ) {
        if let Event::Turn { player_id, .. } = *event {
            self.turns.borrow_mut().push(player_id);
        }
    }
}

#[test]
fn dead_player_never_gets_another_turn() {
    let turns = Rc::new(RefCell::new(Vec::new()));
    let mut world = World::new();
    let a = world.add_player(Player::new("A".to_string(), 50, 5));
    let b = world.add_player(Player::new("B".to_string(), 5, 1));
    let c = world.add_player(Player::new("C".to_string(), 50, 5));
    world.add_buff(Box::new(TurnSpy {
        turns: turns.clone(),
    }));
    for id in [a, b, c] {
        world.add_buff(Box::new(Abilities::new(id, vec![Box::new(Attack)])));
    }

    world.run();

    assert!(world.is_end());
    assert_eq!(
        turns.borrow().iter().filter(|&&id| id == b).count(),
        1,
        "B 只应在死亡当轮行动一次，死亡后不得再获得回合"
    );
}
