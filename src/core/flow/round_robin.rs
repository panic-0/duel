use super::super::{event::Event, state::GameState};
use super::FlowDriver;

/// 默认流程：DuelStart 后按回合推进，回合内按玩家 ID 轮转行动
#[derive(Debug, Default)]
pub struct RoundRobinFlow;

impl FlowDriver for RoundRobinFlow {
    fn advance(&mut self, state: &GameState, event: &Event) -> Vec<Event> {
        match event {
            Event::DuelStart => vec![Event::RoundStart { round: 1 }],
            Event::RoundStart { round } => state
                .get_players()
                .keys()
                .next()
                .map(|first_player_id| Event::BeforeTurn {
                    round: *round,
                    player_id: *first_player_id,
                })
                .into_iter()
                .collect(),
            Event::BeforeTurn { round, player_id } => vec![Event::Turn {
                round: *round,
                player_id: *player_id,
            }],
            Event::Turn { round, player_id } => vec![Event::AfterTurn {
                round: *round,
                player_id: *player_id,
            }],
            Event::AfterTurn { round, player_id } => {
                match state.get_next_player_not_around(*player_id) {
                    Some(next_player_id) => vec![Event::BeforeTurn {
                        round: *round,
                        player_id: next_player_id,
                    }],
                    None => vec![Event::RoundEnd { round: *round }],
                }
            }
            Event::RoundEnd { round } => vec![Event::RoundStart { round: *round + 1 }],
            _ => vec![],
        }
    }
}
