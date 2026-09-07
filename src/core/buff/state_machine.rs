use super::super::{
    command::{ApplyEvent, Commands},
    event::{Event, EventType},
    world::World,
};
use super::{Buff, Priority};

#[derive(Debug)]
pub struct StateMachine;

impl Buff for StateMachine {
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        [
            EventType::DuelStart,
            EventType::RoundStart,
            EventType::BeforeTurn,
            EventType::Turn,
            EventType::AfterTurn,
            EventType::RoundEnd,
        ]
        .into_iter()
        .map(|event_type| (event_type, Priority::StateMachine))
        .collect()
    }

    fn on_event(
        &self,
        event: &mut Event,
        world: &World,
        commands: &mut Commands,
        _buff_id: super::super::BuffId,
    ) {
        let next_event = match event {
            Event::DuelStart => Some(Event::RoundStart { round: 1 }),
            Event::RoundStart { round } => {
                world
                    .get_players()
                    .keys()
                    .next()
                    .map(|first_player_id| Event::BeforeTurn {
                        round: *round,
                        player_id: *first_player_id,
                    })
            }
            Event::BeforeTurn { round, player_id } => Some(Event::Turn {
                round: *round,
                player_id: *player_id,
            }),
            Event::Turn { round, player_id } => Some(Event::AfterTurn {
                round: *round,
                player_id: *player_id,
            }),
            Event::AfterTurn { round, player_id } => {
                if let Some(next_player_id) = world.get_next_player_not_around(*player_id) {
                    Some(Event::BeforeTurn {
                        round: *round,
                        player_id: next_player_id,
                    })
                } else {
                    Some(Event::RoundEnd { round: *round })
                }
            }
            Event::RoundEnd { round } => Some(Event::RoundStart { round: *round + 1 }),
            _ => None,
        };

        if let Some(next_event) = next_event {
            commands.push(ApplyEvent { event: next_event });
        }
    }
}
