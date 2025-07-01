use super::super::modifier::HpModifier;
use super::*;

#[derive(Debug)]
pub struct Attack;

impl Attack {
    fn get_target(&self, source_id: PlayerId, world: &World) -> Option<PlayerId> {
        world.get_next_player_around(source_id)
    }
}

impl Ability for Attack {
    fn apply(&self, source_id: PlayerId, world: &World, commands: &mut Commands) {
        let Some(source) = world.get_player(source_id) else {
            return;
        };

        if let Some(target_id) = self.get_target(source_id, world) {
            let Some(target) = world.get_player(target_id) else {
                return;
            };

            println!(
                "{} 对 {} 进行了普通攻击",
                source.name().blue(),
                target.name().blue()
            );

            commands.push(HpModifier::damage(target_id, source.attack()));
        }
    }
}
