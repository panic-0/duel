use duel::core::{player::Player, world::World, PlayerId};

pub(super) struct GameBuilder {
    world: World,
}

impl GameBuilder {
    pub fn new() -> Self {
        Self {
            world: World::new(),
        }
    }

    pub fn add_player(mut self, name: &str, hp: u64, attack: u64) -> (Self, PlayerId) {
        let player = Player::new(name.to_string(), hp, attack);
        let player_id = self.world.add_player(player);
        (self, player_id)
    }

    pub fn build(self) -> World {
        self.world
    }
}

impl Default for GameBuilder {
    fn default() -> Self {
        Self::new()
    }
}
