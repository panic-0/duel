use super::PlayerId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Checkpoint {
    DuelStart,
    RoundStart,
    TurnStart,
    ActionEnd,
    TurnEnd,
    RoundEnd,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Event {
    // 状态事件
    DuelStart,
    HpChanged {
        target_id: PlayerId,
        old_hp: u64,
        new_hp: u64,
    },
    Checkpoint {
        phase: Checkpoint,
        round: Option<u32>,
    },
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

    // 行为事件
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
    // 状态事件
    DuelStart,
    HpChanged,
    Checkpoint,
    RoundStart,
    BeforeTurn,
    Turn,
    AfterTurn,
    RoundEnd,

    // 行为事件
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
            Event::HpChanged { .. } => EventType::HpChanged,
            Event::Checkpoint { .. } => EventType::Checkpoint,
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
