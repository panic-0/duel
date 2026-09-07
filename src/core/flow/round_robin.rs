use super::super::{event::Event, state::GameState, PlayerId};
use super::FlowDriver;
use std::ops::Bound::{Excluded, Unbounded};

/// 默认流程：DuelStart 后按回合推进，回合内按玩家 ID 轮转行动
#[derive(Debug, Default)]
pub struct RoundRobinFlow;

impl RoundRobinFlow {
    fn next_turn(state: &GameState, round: u32, player_id: PlayerId) -> Event {
        state
            .get_players()
            .range((Excluded(player_id), Unbounded))
            .find(|(_, player)| player.is_alive())
            .map(|(&player_id, _)| Event::BeforeTurn { round, player_id })
            .unwrap_or(Event::RoundEnd { round })
    }
}

impl FlowDriver for RoundRobinFlow {
    fn advance(&mut self, state: &GameState, event: &Event) -> Vec<Event> {
        match event {
            Event::DuelStart => vec![Event::RoundStart { round: 1 }],
            Event::RoundStart { round } => state
                .get_players()
                .iter()
                .find(|(_, player)| player.is_alive())
                .map(|(first_player_id, _)| Event::BeforeTurn {
                    round: *round,
                    player_id: *first_player_id,
                })
                .into_iter()
                .collect(),
            // BeforeTurn 引出的全部反应已结算，此处按最终状态决定是否行动。
            Event::BeforeTurn { round, player_id }
                if !state
                    .get_player(*player_id)
                    .is_some_and(|player| player.is_alive()) =>
            {
                vec![Self::next_turn(state, *round, *player_id)]
            }
            Event::BeforeTurn { round, player_id } => vec![Event::Turn {
                round: *round,
                player_id: *player_id,
            }],
            Event::Turn { round, player_id } => vec![Event::AfterTurn {
                round: *round,
                player_id: *player_id,
            }],
            Event::AfterTurn { round, player_id } => {
                vec![Self::next_turn(state, *round, *player_id)]
            }
            Event::RoundEnd { round } => vec![Event::RoundStart { round: *round + 1 }],
            _ => vec![],
        }
    }
}
