#[derive(Debug, Clone)]
pub struct Player {
    name: String,
    hp: u64,
    max_hp: u64,
    attack: u64,
}

impl Player {
    pub fn new(name: String, hp: u64, attack: u64) -> Self {
        Self {
            name,
            hp,
            max_hp: hp,
            attack,
        }
    }

    // 只读访问
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn hp(&self) -> u64 {
        self.hp
    }

    pub fn max_hp(&self) -> u64 {
        self.max_hp
    }

    pub fn attack(&self) -> u64 {
        self.attack
    }

    pub fn is_alive(&self) -> bool {
        self.hp > 0
    }

    pub fn hp_percentage(&self) -> f64 {
        self.hp as f64 / self.max_hp as f64
    }

    // 带校验的写入
    /// 仅限对局开始前的初始配置；运行期的生命变化必须走受控提交
    /// （ExecutionContext::modify_hp / submit_damage），否则不会产生基础事件。
    pub fn set_hp(&mut self, hp: u64) {
        self.hp = hp.min(self.max_hp);
    }

    pub fn modify_hp(&mut self, modifier: i64) -> u64 {
        if modifier < 0 {
            self.damage(modifier.unsigned_abs());
        } else {
            self.heal(modifier as u64);
        }
        self.hp
    }

    pub(crate) fn damage(&mut self, amount: u64) -> u64 {
        self.hp = self.hp.saturating_sub(amount);
        self.hp
    }

    pub(crate) fn heal(&mut self, amount: u64) -> u64 {
        self.hp = self.hp.saturating_add(amount).min(self.max_hp);
        self.hp
    }

    pub fn heal_to_full(&mut self) {
        self.hp = self.max_hp;
    }
}

#[cfg(test)]
mod tests {
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
}
