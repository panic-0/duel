use duel::core::{engine::BattleEngine, player::Player, PlayerId};

pub(super) struct GameBuilder {
    world: BattleEngine,
}

impl GameBuilder {
    pub fn new() -> Self {
        Self {
            world: BattleEngine::new(),
        }
    }

    pub fn add_player(mut self, name: &str, hp: u64, attack: u64) -> (Self, PlayerId) {
        let player = Player::new(name.to_string(), hp, attack);
        let player_id = self.world.add_player(player);
        (self, player_id)
    }

    pub fn build(self) -> BattleEngine {
        self.world
    }
}

impl Default for GameBuilder {
    fn default() -> Self {
        Self::new()
    }
}
