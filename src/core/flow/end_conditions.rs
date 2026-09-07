use super::super::{event::Event, state::GameState};
use super::{EndCondition, GameResult, MAX_ROUNDS};

/// 达到回合上限判平局
#[derive(Debug)]
pub struct RoundLimit {
    max_rounds: u32,
}

impl RoundLimit {
    pub fn new(max_rounds: u32) -> Self {
        RoundLimit { max_rounds }
    }
}

impl Default for RoundLimit {
    fn default() -> Self {
        RoundLimit::new(MAX_ROUNDS)
    }
}

impl EndCondition for RoundLimit {
    fn check(&mut self, _state: &GameState, event: &Event) -> Option<GameResult> {
        match event {
            Event::RoundStart { round } if *round > self.max_rounds => Some(GameResult::Draw),
            _ => None,
        }
    }
}

/// 剩最后一名玩家时结束。见到 DuelStart 才开始判定，
/// 未开局的世界（如直接分发事件的测试环境）不会被误判为结束。
#[derive(Debug, Default)]
pub struct LastManStanding {
    started: bool,
}

impl EndCondition for LastManStanding {
    fn check(&mut self, state: &GameState, event: &Event) -> Option<GameResult> {
        if matches!(event, Event::DuelStart) {
            self.started = true;
            return None;
        }
        (self.started && state.get_players().len() <= 1).then_some(GameResult::LastStanding)
    }
}
