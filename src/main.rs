pub mod core;

use core::{
    ability::{Abilities, Attack},
    player::Player,
    world::World,
    PlayerId,
};

pub struct GameBuilder {
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

    pub fn give_basic_abilities(mut self, player_id: PlayerId) -> Self {
        self.world
            .add_buff(Box::new(Abilities::new(player_id, vec![Box::new(Attack)])));
        self
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

fn main() {
    let builder = GameBuilder::new();
    let (builder, player1_id) = builder.add_player("Player1", 15, 10);
    let (builder, player2_id) = builder.add_player("Player2", 28, 8);
    let mut world = builder
        .give_basic_abilities(player1_id)
        .give_basic_abilities(player2_id)
        .build();

    world.add_buff(Box::new(core::buff::Revival::new(player1_id)));
    world.add_buff(Box::new(core::buff::DamageReduction::new(player2_id, 0.2)));

    world.run();
}
