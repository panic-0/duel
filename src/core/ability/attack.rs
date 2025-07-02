use super::super::command::{AddBuff, ApplyEvent, RemoveBuff};
use super::super::modifier::HpModifier;
use super::*;
use colored::Colorize;

#[derive(Debug)]
pub struct Attack;

impl Attack {
    fn get_target(&self, source_id: PlayerId, world: &World) -> Option<PlayerId> {
        world.get_next_player_around(source_id)
    }
}

#[derive(Debug)]
struct AttackBuff {
    source_id: PlayerId,
    target_id: PlayerId,
}

impl Buff for AttackBuff {
    fn buff_type(&self) -> BuffType {
        BuffType::Attack
    }

    fn on_event(&self, event: &mut Event, world: &World, commands: &mut Commands, buff_id: BuffId) {
        let Some(source) = world.get_player(self.source_id) else {
            return;
        };
        let Some(target) = world.get_player(self.target_id) else {
            return;
        };
        match *event {
            Event::BeforePlayerAttack {
                source_id,
                target_id,
            } => {
                if source_id != self.source_id || target_id != self.target_id {
                    return;
                }

                commands.push(ApplyEvent {
                    event: Event::PlayerAttack {
                        source_id,
                        target_id,
                        damage: source.attack(),
                    },
                });
            }
            Event::PlayerAttack {
                source_id,
                target_id,
                damage,
            } => {
                if source_id != self.source_id || target_id != self.target_id {
                    return;
                }

                commands.push(RemoveBuff { id: buff_id });
                println!(
                    "{} 对 {} 进行了普通攻击",
                    source.name().blue(),
                    target.name().blue()
                );
                commands.push(HpModifier::damage(target_id, damage));

                commands.push(ApplyEvent {
                    event: Event::AfterPlayerAttack {
                        source_id,
                        target_id,
                        damage,
                    },
                });
            }
            _ => {}
        }
    }
}

impl Ability for Attack {
    fn apply(&self, source_id: PlayerId, world: &World, commands: &mut Commands) {
        if let Some(target_id) = self.get_target(source_id, world) {
            commands.push(AddBuff {
                buff: Box::new(AttackBuff {
                    source_id,
                    target_id,
                }),
            });
            commands.push(ApplyEvent {
                event: Event::BeforePlayerAttack {
                    source_id,
                    target_id,
                },
            });
        }
    }
}
