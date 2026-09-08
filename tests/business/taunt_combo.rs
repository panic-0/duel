//! 嘲讽目标选择与连击技能的行为测试。

use duel::core::{
    add_taunt, player::Player, Abilities, AttackOperation, Combo, TurnOperation, World,
};

#[test]
fn taunt_redirects_attack_to_the_earliest_living_taunter() {
    let mut world = World::new();
    let attacker = world.add_player(Player::new("attacker".into(), 20, 4));
    let ordinary_target = world.add_player(Player::new("ordinary".into(), 20, 1));
    let taunter = world.add_player(Player::new("taunter".into(), 20, 1));
    add_taunt(&mut world, taunter);

    world
        .execute(AttackOperation::new(attacker))
        .expect("攻击应完成");

    assert_eq!(world.get_player(ordinary_target).unwrap().hp(), 20);
    assert_eq!(world.get_player(taunter).unwrap().hp(), 16);
}

#[test]
fn dead_taunter_is_ignored_and_attack_falls_back_to_normal_targeting() {
    let mut world = World::new();
    let attacker = world.add_player(Player::new("attacker".into(), 20, 4));
    let ordinary_target = world.add_player(Player::new("ordinary".into(), 20, 1));
    let taunter = world.add_player(Player::new("taunter".into(), 20, 1));
    add_taunt(&mut world, taunter);
    assert!(world.set_initial_hp(taunter, 0));

    world
        .execute(AttackOperation::new(attacker))
        .expect("攻击应完成");

    assert_eq!(world.get_player(ordinary_target).unwrap().hp(), 16);
}

#[test]
fn combo_performs_multiple_attacks_as_one_normal_action() {
    let mut world = World::new();
    let attacker = world.add_player(Player::new("attacker".into(), 20, 3));
    let target = world.add_player(Player::new("target".into(), 10, 1));
    world.add_data(
        Some(attacker),
        Abilities::new(attacker, vec![Box::new(Combo::new(2))]),
    );

    world
        .execute(TurnOperation {
            round: 1,
            player_id: attacker,
        })
        .expect("连击应完成");

    assert_eq!(world.get_player(target).unwrap().hp(), 4);
}
