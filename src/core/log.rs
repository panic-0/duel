use super::{world::World, PlayerId};

/// 引擎产生的结构化日志：只携带 ID 和数值，展示（名字、颜色、文案）由打印端负责
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogEntry {
    Attack {
        source_id: PlayerId,
        target_id: PlayerId,
    },
    DamageReduced {
        target_id: PlayerId,
        original: u64,
        reduced: u64,
    },
    Damage {
        target_id: PlayerId,
        amount: u64,
        hp: u64,
        max_hp: u64,
    },
    Heal {
        target_id: PlayerId,
        amount: u64,
        hp: u64,
        max_hp: u64,
    },
    Revival {
        player_id: PlayerId,
    },
    Death {
        player_id: PlayerId,
    },
    Draw,
}

/// 日志回调：拿到 `&World` 以便打印时解析玩家名
pub type LogCallback = Box<dyn Fn(&World, &LogEntry)>;

/// 日志通道。默认为空（静音），安装回调后世界在运行时向外发射结构化日志
#[derive(Default)]
pub struct Logger(Option<LogCallback>);

impl Logger {
    pub fn new(f: LogCallback) -> Self {
        Logger(Some(f))
    }

    pub fn emit(&self, world: &World, entry: &LogEntry) {
        if let Some(f) = &self.0 {
            f(world, entry);
        }
    }
}

impl std::fmt::Debug for Logger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Logger(..)")
    }
}
