use super::{
    buff::{Buff, Priority},
    command::Commands,
    event::{Event, EventType},
    log::{LogEntry, Logger},
    player::Player,
    BuffId, PlayerId,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

/// 单场对局的最大回合数，防止无限对局挂起
pub const MAX_ROUNDS: u32 = 10000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EventRegistration {
    priority: Priority,
    buff_id: BuffId,
}

impl PartialOrd for EventRegistration {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::cmp::Ord for EventRegistration {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| self.buff_id.cmp(&other.buff_id))
    }
}

#[derive(Debug)]
pub struct World {
    players: BTreeMap<PlayerId, Player>,
    player_id_counter: PlayerId,
    buffs: BTreeMap<BuffId, Box<dyn Buff>>,
    buff_id_counter: BuffId,
    event_registry: HashMap<EventType, BTreeSet<EventRegistration>>,
    event_queue: VecDeque<Event>,
    logger: Logger,
    end: bool,
}

impl World {
    pub fn new() -> Self {
        World {
            players: BTreeMap::new(),
            player_id_counter: 0,
            buffs: BTreeMap::new(),
            buff_id_counter: 0,
            event_registry: HashMap::new(),
            event_queue: VecDeque::new(),
            logger: Logger::default(),
            end: false,
        }
    }

    pub fn add_player(&mut self, player: Player) -> PlayerId {
        let id = self.player_id_counter;
        self.player_id_counter += 1;
        self.players.insert(id, player);
        id
    }

    pub fn remove_player(&mut self, id: PlayerId) -> Option<Player> {
        self.players.remove(&id)
    }

    pub fn get_players(&self) -> &BTreeMap<PlayerId, Player> {
        &self.players
    }

    pub fn get_player(&self, id: PlayerId) -> Option<&Player> {
        self.players.get(&id)
    }

    pub fn get_player_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        self.players.get_mut(&id)
    }

    pub fn add_buff(&mut self, buff: Box<dyn Buff>) -> BuffId {
        let id = self.buff_id_counter;
        self.buff_id_counter += 1;

        for (event_type, priority) in buff.subscriptions() {
            let registrations = self.event_registry.entry(event_type).or_default();
            registrations.insert(EventRegistration {
                priority,
                buff_id: id,
            });
        }

        self.buffs.insert(id, buff);
        id
    }

    pub fn remove_buff(&mut self, id: BuffId) -> Option<Box<dyn Buff>> {
        if let Some(buff) = self.buffs.remove(&id) {
            for registrations in self.event_registry.values_mut() {
                registrations.retain(|reg| reg.buff_id != id);
            }
            Some(buff)
        } else {
            None
        }
    }

    pub fn get_buffs(&self) -> &BTreeMap<BuffId, Box<dyn Buff>> {
        &self.buffs
    }

    pub fn get_buff(&self, id: BuffId) -> Option<&dyn Buff> {
        self.buffs.get(&id).map(|b| b.as_ref())
    }

    pub fn get_buff_mut(&mut self, id: BuffId) -> Option<&mut Box<dyn Buff>> {
        self.buffs.get_mut(&id)
    }

    pub fn set_logger(&mut self, logger: Logger) {
        self.logger = logger;
    }

    pub fn log(&self, entry: LogEntry) {
        self.logger.emit(self, &entry);
    }

    pub fn queue_event(&mut self, event: Event) {
        self.event_queue.push_back(event);
    }

    pub fn apply_event(&mut self, event: &mut Event) {
        if self.is_end() {
            return;
        }
        let mut commands = Commands::default();

        if let Some(registrations) = self.event_registry.get(&event.event_type()) {
            for registration in registrations {
                if let Some(buff) = self.buffs.get(&registration.buff_id) {
                    buff.on_event(event, self, &mut commands, registration.buff_id);
                }
            }
        }

        commands
            .commands
            .into_iter()
            .for_each(|command| command.apply(self));

        self.advance_flow(event);

        if self.players.len() <= 1 {
            self.end = true;
        }
    }

    /// 引擎内置的全局流程推进：在每个事件处理完毕后生成下一个流程事件。
    /// 相当于原先的 StateMachine buff，但不再占用 buff 列表。
    fn advance_flow(&mut self, event: &mut Event) {
        let next_event = match event {
            Event::DuelStart => Some(Event::RoundStart { round: 1 }),
            Event::RoundStart { round } => {
                self.get_players()
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
                if let Some(next_player_id) = self.get_next_player_not_around(*player_id) {
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
            self.queue_event(next_event);
        }
    }

    pub fn is_end(&self) -> bool {
        self.end
    }
}

impl World {
    pub fn get_next_player_not_around(&self, id: PlayerId) -> Option<PlayerId> {
        self.players.range(id + 1..).next().map(|(id, _)| *id)
    }

    pub fn get_next_player_around(&self, id: PlayerId) -> Option<PlayerId> {
        self.players
            .range(id + 1..)
            .next()
            .or_else(|| self.players.range(..id).next())
            .map(|(id, _)| *id)
    }

    /// 完整跑一局：播下 DuelStart 后泵事件队列直到结束
    pub fn run(&mut self) {
        self.apply_event(&mut Event::DuelStart);
        self.pump();
    }

    /// 泵事件队列直到结束或排空，可单独调用来分步驱动对局
    pub fn pump(&mut self) {
        while let Some(mut event) = self.event_queue.pop_front() {
            if self.is_end() {
                break;
            }
            if let Event::RoundStart { round } = event {
                if round > MAX_ROUNDS {
                    self.log(LogEntry::Draw);
                    self.end = true;
                    break;
                }
            }
            self.apply_event(&mut event);
        }
    }
}

impl Default for World {
    fn default() -> Self {
        World::new()
    }
}

impl World {
    pub fn get_registry_stats(&self) -> (usize, usize, usize) {
        let total_events = self.event_registry.len();
        let total_registrations: usize = self.event_registry.values().map(|set| set.len()).sum();
        let total_buffs = self.buffs.len();
        (total_events, total_registrations, total_buffs)
    }

    pub fn get_event_registration_count(&self, event_type: EventType) -> usize {
        self.event_registry
            .get(&event_type)
            .map(|set| set.len())
            .unwrap_or(0)
    }

    pub fn validate_registry_consistency(&self) -> bool {
        for registrations in self.event_registry.values() {
            for registration in registrations {
                if !self.buffs.contains_key(&registration.buff_id) {
                    return false;
                }
            }
        }
        true
    }
}
