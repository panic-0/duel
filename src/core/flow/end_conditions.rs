use super::super::{event::Event, state::GameState};
use super::{EndCondition, GameResult, MAX_ROUNDS};

/// 完成最后一回合及其结束效果后判平局；上限为 0 时只完成开局结算。
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
            Event::DuelStart if self.max_rounds == 0 => Some(GameResult::Draw),
            Event::RoundEnd { round } if *round >= self.max_rounds => Some(GameResult::Draw),
            _ => None,
        }
    }
}

/// 剩最后一名玩家时结束。完成 DuelStart 的结算后才开始判定，
/// 未开局的世界（如直接分发事件的测试环境）不会被误判为结束。
#[derive(Debug, Default)]
pub struct LastManStanding {
    started: bool,
}

impl EndCondition for LastManStanding {
    fn check(&mut self, state: &GameState, event: &Event) -> Option<GameResult> {
        if matches!(event, Event::DuelStart) {
            self.started = true;
        }
        (self.started && state.get_players().len() <= 1).then_some(GameResult::LastStanding)
    }
}
