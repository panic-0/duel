use super::PlayerId;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Event {
    // State events
    DuelStart,
    RoundStart {
        round: u32,
    },
    BeforeTurn {
        round: u32,
        player_id: PlayerId,
    },
    Turn {
        round: u32,
        player_id: PlayerId,
    },
    AfterTurn {
        round: u32,
        player_id: PlayerId,
    },
    RoundEnd {
        round: u32,
    },

    // Action events
    BeforePlayerAttack {
        source_id: PlayerId,
        target_id: PlayerId,
    },
    PlayerAttack {
        source_id: PlayerId,
        target_id: PlayerId,
        damage: u64,
    },
    AfterPlayerAttack {
        source_id: PlayerId,
        target_id: PlayerId,
        damage: u64,
    },

    BeforePlayerDeath(PlayerId),
    AfterPlayerDeath(PlayerId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventType {
    // State events
    DuelStart,
    RoundStart,
    BeforeTurn,
    Turn,
    AfterTurn,
    RoundEnd,

    // Action events
    BeforePlayerAttack,
    PlayerAttack,
    AfterPlayerAttack,
    BeforePlayerDeath,
    AfterPlayerDeath,
}

impl Event {
    pub fn event_type(&self) -> EventType {
        match self {
            Event::DuelStart => EventType::DuelStart,
            Event::RoundStart { .. } => EventType::RoundStart,
            Event::BeforeTurn { .. } => EventType::BeforeTurn,
            Event::Turn { .. } => EventType::Turn,
            Event::AfterTurn { .. } => EventType::AfterTurn,
            Event::RoundEnd { .. } => EventType::RoundEnd,
            Event::BeforePlayerAttack { .. } => EventType::BeforePlayerAttack,
            Event::PlayerAttack { .. } => EventType::PlayerAttack,
            Event::AfterPlayerAttack { .. } => EventType::AfterPlayerAttack,
            Event::BeforePlayerDeath(_) => EventType::BeforePlayerDeath,
            Event::AfterPlayerDeath(_) => EventType::AfterPlayerDeath,
        }
    }
}
