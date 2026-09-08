//! 战斗事件与事件种类。Event 是不可变事实，EventKind 用于订阅和路由。

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
    // 注意：攻击类事件中的 target_id 均指“本次攻击最初选定的目标”，
    // 即使后续参数规则把实际伤害重定向到他人也不改写；
    // 实际承受伤害的角色以 Damage 提交返回的 HpChange.target_id 为准。
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
pub enum EventKind {
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
    /// Component 被显式移除或因 owner 生命周期结束而销毁。
    ComponentDestroyed,
}

impl Event {
    pub fn kind(&self) -> EventKind {
        match self {
            Event::DuelStart => EventKind::DuelStart,
            Event::HpChanged { .. } => EventKind::HpChanged,
            Event::Checkpoint { .. } => EventKind::Checkpoint,
            Event::RoundStart { .. } => EventKind::RoundStart,
            Event::BeforeTurn { .. } => EventKind::BeforeTurn,
            Event::Turn { .. } => EventKind::Turn,
            Event::AfterTurn { .. } => EventKind::AfterTurn,
            Event::RoundEnd { .. } => EventKind::RoundEnd,
            Event::BeforePlayerAttack { .. } => EventKind::BeforePlayerAttack,
            Event::PlayerAttack { .. } => EventKind::PlayerAttack,
            Event::AfterPlayerAttack { .. } => EventKind::AfterPlayerAttack,
            Event::BeforePlayerDeath(_) => EventKind::BeforePlayerDeath,
            Event::AfterPlayerDeath(_) => EventKind::AfterPlayerDeath,
        }
    }
}
