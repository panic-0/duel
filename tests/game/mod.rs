//! 完整对局、默认规则装配与自定义流程。

use crate::support::log_sink;
use duel::core::{
    business::{
        damage_reduction::{attach_damage_reduction, register_damage_reduction_rule},
        revival::{attach_revival, register_revival_system},
        skills::Abilities,
    },
    event::Event,
    install_default_rules,
    log::LogEntry,
    operation::{completed, ActionContext, Operation, OperationError, OperationOutcome},
    player::Player,
    Attack, BattleEngine, DuelRunner,
};

#[test]
fn full_game_runs_to_expected_outcome() {
    let mut world = BattleEngine::new();
    let p1 = world.add_player(Player::new("Player1".to_string(), 15, 10));
    let p2 = world.add_player(Player::new("Player2".to_string(), 28, 8));
    world.attach_component(Some(p1), Abilities::new(p1, vec![Box::new(Attack)]));
    register_revival_system(&mut world);
    attach_revival(&mut world, p1);
    world.attach_component(Some(p2), Abilities::new(p2, vec![Box::new(Attack)]));
    register_damage_reduction_rule(&mut world);
    attach_damage_reduction(&mut world, p2, p2, 0.2);
    install_default_rules(&mut world);

    world.run().expect("对局应正常结束");

    assert!(world.is_end());
    assert!(world.player(p1).is_none(), "Player1 应已死亡移除");
    let winner = world.player(p2).unwrap();
    assert_eq!(winner.hp(), 4);
    assert!(winner.is_alive());
}

#[test]
fn round_limit_stops_unwinnable_game() {
    let (entries, logger) = log_sink();
    let mut world = BattleEngine::new();
    world.set_logger(logger);

    let p1 = world.add_player(Player::new("A".to_string(), 10, 2));
    let p2 = world.add_player(Player::new("B".to_string(), 10, 2));
    world.attach_component(Some(p1), Abilities::new(p1, vec![Box::new(Attack)]));
    register_damage_reduction_rule(&mut world);
    attach_damage_reduction(&mut world, p1, p1, 1.0);
    world.attach_component(Some(p2), Abilities::new(p2, vec![Box::new(Attack)]));
    attach_damage_reduction(&mut world, p2, p2, 1.0);
    install_default_rules(&mut world);

    world.run().expect("对局应正常结束");

    assert!(world.is_end(), "无限对局应被回合上限终止");
    assert!(world.player(p1).is_some(), "双方应都存活");
    assert!(world.player(p2).is_some());
    assert!(entries.borrow().contains(&LogEntry::Draw), "应记录平局日志");
}

/// 只发布开局通知、不安排任何回合的自定义流程
#[derive(Debug)]
struct OpeningOnly;

impl Operation for OpeningOnly {
    fn execute(
        self: Box<Self>,
        context: &mut ActionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.try_publish(Event::DuelStart)?;
        completed()
    }
}

#[test]
fn custom_flow_operation_replaces_round_structure() {
    let mut world = BattleEngine::new();
    world.add_player(Player::new("A".to_string(), 10, 5));
    world.add_player(Player::new("B".to_string(), 10, 5));

    world.execute(OpeningOnly).expect("自定义流程应正常执行");

    // 流程只发布了 DuelStart，对局停留在开局之后，不判结束
    assert!(!world.is_end());
    assert!(world.player(0).is_some());
    assert!(world.player(1).is_some());
}

#[test]
fn three_player_game_ends_with_single_survivor() {
    let mut world = BattleEngine::new();
    for name in ["A", "B", "C"] {
        world.add_player(Player::new(name.to_string(), 30, 10));
    }
    for id in 0..3 {
        world.attach_component(Some(id), Abilities::new(id, vec![Box::new(Attack)]));
    }
    install_default_rules(&mut world);

    world.run().expect("对局应正常结束");

    assert!(world.is_end(), "三人对局应打到只剩一人");
    let survivors: Vec<_> = world.players().keys().copied().collect();
    assert_eq!(survivors.len(), 1);
}

#[test]
fn custom_round_limit_ends_game_early() {
    let mut world = BattleEngine::new();
    world.add_player(Player::new("A".to_string(), 100, 5));
    world.add_player(Player::new("B".to_string(), 100, 5));
    world.attach_component(Some(0), Abilities::new(0, vec![Box::new(Attack)]));
    world.attach_component(Some(1), Abilities::new(1, vec![Box::new(Attack)]));

    world.run_with_max_rounds(2).expect("对局应正常结束");

    assert!(world.is_end(), "应因回合上限提前结束");
    assert!(world.player(0).is_some(), "2 回合内双方都应存活");
    assert!(world.player(1).is_some());
}
