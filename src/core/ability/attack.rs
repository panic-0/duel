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
struct AttackSettlementBuff {
    source_id: PlayerId,
}

impl Buff for AttackSettlementBuff {
    fn buff_type(&self) -> BuffType {
        BuffType::AttackSettlement
    }

    fn on_event(&self, event: &mut Event, world: &World, commands: &mut Commands, buff_id: BuffId) {
        if let Event::PlayerAttack {
            source_id,
            target_id,
            damage,
        } = *event
        {
            if source_id == self.source_id {
                let Some(source) = world.get_player(source_id) else {
                    return;
                };
                let Some(target) = world.get_player(target_id) else {
                    return;
                };

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
        }
    }
}

impl Ability for Attack {
    fn apply(&self, source_id: PlayerId, world: &World, commands: &mut Commands) {
        let Some(source) = world.get_player(source_id) else {
            return;
        };

        if let Some(target_id) = self.get_target(source_id, world) {
            commands.push(ApplyEvent {
                event: Event::BeforePlayerAttack {
                    source_id,
                    target_id,
                },
            });

            commands.push(AddBuff {
                buff: Box::new(AttackSettlementBuff { source_id }),
            });

            commands.push(ApplyEvent {
                event: Event::PlayerAttack {
                    source_id,
                    target_id,
                    damage: source.attack(),
                },
            });
        }
    }
}
