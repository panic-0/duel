use colored::Colorize;

use super::super::{
    command::{Commands, RemoveBuff},
    event::Event,
    modifier::HpModifier,
    world::World,
};
use super::{Buff, BuffType, PlayerId};

#[derive(Debug)]
pub struct Revival {
    pub source_id: PlayerId,
}

impl Revival {
    pub fn new(source_id: PlayerId) -> Self {
        Revival { source_id }
    }
}

impl Buff for Revival {
    fn buff_type(&self) -> BuffType {
        BuffType::Revival
    }

    fn on_event(
        &self,
        event: &mut Event,
        world: &World,
        commands: &mut Commands,
        buff_id: super::super::BuffId,
    ) {
        match *event {
            Event::BeforePlayerDeath(player_id) if player_id == self.source_id => {
                if let Some(player) = world.get_player(self.source_id) {
                    println!("{} 复活了！", player.name().blue());
                    commands.push(RemoveBuff { id: buff_id });
                    commands.push(HpModifier::heal(self.source_id, player.max_hp() / 2));
                }
            }
            _ => {}
        }
    }
}
