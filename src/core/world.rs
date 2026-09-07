use super::{
    buff::{Buff, Priority},
    command::Commands,
    event::{Event, EventType},
    flow::{
        EndCondition, FlowDriver, GameResult, LastManStanding, RoundLimit, RoundRobinFlow,
        MAX_ROUNDS,
    },
    log::{LogEntry, Logger},
    player::Player,
    state::GameState,
    BuffId, PlayerId,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

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

/// 引擎：负责事件泵、按订阅分发与命令执行。
/// 游戏状态在 [`GameState`]，回合流程与结束条件由可插拔的
/// [`FlowDriver`] / [`EndCondition`] 决定。
#[derive(Debug)]
pub struct World {
    state: GameState,
    buffs: BTreeMap<BuffId, Box<dyn Buff>>,
    buff_id_counter: BuffId,
    event_registry: HashMap<EventType, BTreeSet<EventRegistration>>,
    event_queue: VecDeque<Event>,
    flow: Box<dyn FlowDriver>,
    end_conditions: Vec<Box<dyn EndCondition>>,
    end: bool,
}

impl World {
    pub fn new() -> Self {
        World {
            state: GameState::new(),
            buffs: BTreeMap::new(),
            buff_id_counter: 0,
            event_registry: HashMap::new(),
            event_queue: VecDeque::new(),
            flow: Box::new(RoundRobinFlow),
            end_conditions: vec![
                Box::new(RoundLimit::new(MAX_ROUNDS)),
                Box::new(LastManStanding::default()),
            ],
            end: false,
        }
    }

    /// 替换流程驱动，决定对局的回合/轮转结构
    pub fn set_flow(&mut self, flow: Box<dyn FlowDriver>) {
        self.flow = flow;
    }

    /// 追加结束条件（默认已装配回合上限与最后一人生还）
    pub fn add_end_condition(&mut self, condition: Box<dyn EndCondition>) {
        self.end_conditions.push(condition);
    }

    pub fn add_player(&mut self, player: Player) -> PlayerId {
        self.state.add_player(player)
    }

    pub fn remove_player(&mut self, id: PlayerId) -> Option<Player> {
        self.state.remove_player(id)
    }

    pub fn get_players(&self) -> &BTreeMap<PlayerId, Player> {
        self.state.get_players()
    }

    pub fn get_player(&self, id: PlayerId) -> Option<&Player> {
        self.state.get_player(id)
    }

    pub fn get_player_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        self.state.get_player_mut(id)
    }

    pub fn get_next_player_not_around(&self, id: PlayerId) -> Option<PlayerId> {
        self.state.get_next_player_not_around(id)
    }

    pub fn get_next_player_around(&self, id: PlayerId) -> Option<PlayerId> {
        self.state.get_next_player_around(id)
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
        self.state.set_logger(logger);
    }

    pub fn log(&self, entry: LogEntry) {
        self.state.log(entry);
    }

    pub fn queue_event(&mut self, event: Event) {
        self.event_queue.push_back(event);
    }

    pub fn apply_event(&mut self, event: &mut Event) {
        if self.is_end() {
            return;
        }

        // 结束条件在分发前检查，命中的事件不再进入监听者
        if let Some(result) = self.check_end_conditions(event) {
            self.apply_end(result);
            return;
        }

        let mut commands = Commands::default();

        if let Some(registrations) = self.event_registry.get(&event.event_type()) {
            for registration in registrations {
                if let Some(buff) = self.buffs.get_mut(&registration.buff_id) {
                    buff.on_event(event, &self.state, &mut commands, registration.buff_id);
                }
            }
        }

        commands
            .commands
            .into_iter()
            .for_each(|command| command.apply(self));

        self.resolve_death(event);

        let next_events = self.flow.advance(&self.state, event);
        self.event_queue.extend(next_events);
    }

    /// 死亡结算：`BeforePlayerDeath` 的监听者（如复活）执行完后血量仍为 0，
    /// 死亡坐实——记日志、移除玩家并入队 `AfterPlayerDeath` 终局通知。
    /// 移除必须在此处完成：若延迟到终局通知之后，中间被处理的流程事件
    /// 可能为已死玩家排出行动回合。
    fn resolve_death(&mut self, event: &Event) {
        if let Event::BeforePlayerDeath(id) = *event {
            if self.state.get_player(id).is_some_and(|p| p.hp() == 0) {
                self.log(LogEntry::Death { player_id: id });
                self.state.remove_player(id);
                self.queue_event(Event::AfterPlayerDeath(id));

                // 移除玩家可能直接满足结束条件（如剩最后一人生还）
                if let Some(result) = self.check_end_conditions(event) {
                    self.apply_end(result);
                }
            }
        }
    }

    fn check_end_conditions(&mut self, event: &Event) -> Option<GameResult> {
        self.end_conditions
            .iter_mut()
            .find_map(|condition| condition.check(&self.state, event))
    }

    fn apply_end(&mut self, result: GameResult) {
        if matches!(result, GameResult::Draw) {
            self.log(LogEntry::Draw);
        }
        self.end = true;
    }

    pub fn is_end(&self) -> bool {
        self.end
    }
}

impl World {
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
