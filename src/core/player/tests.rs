use super::*;

#[test]
fn test_player_creation() {
    let player = Player::new("Test".to_string(), 100, 15);
    assert_eq!(player.name(), "Test");
    assert_eq!(player.hp(), 100);
    assert_eq!(player.max_hp(), 100);
    assert_eq!(player.attack(), 15);
    assert!(player.is_alive());
}

#[test]
fn test_player_hp_modification() {
    let mut player = Player::new("Test".to_string(), 100, 15);

    // 伤害
    player.modify_hp(-30);
    assert_eq!(player.hp(), 70);
    assert!(player.is_alive());

    // 治疗
    player.modify_hp(20);
    assert_eq!(player.hp(), 90);

    // 治疗溢出按上限截断
    player.modify_hp(20);
    assert_eq!(player.hp(), 100);

    // 致命伤害
    player.modify_hp(-100);
    assert_eq!(player.hp(), 0);
    assert!(!player.is_alive());

    // 对已死亡玩家继续伤害
    player.modify_hp(-10);
    assert_eq!(player.hp(), 0);
}

#[test]
fn test_player_hp_percentage() {
    let mut player = Player::new("Test".to_string(), 100, 15);
    assert_eq!(player.hp_percentage(), 1.0);

    player.modify_hp(-50);
    assert_eq!(player.hp_percentage(), 0.5);

    player.modify_hp(-50);
    assert_eq!(player.hp_percentage(), 0.0);
}
