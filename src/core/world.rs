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

#[derive(Debug)]
enum SettlementWork {
    Dispatch(Event),
    ConfirmDeath(PlayerId),
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
    /// Some 表示正在结算；命令产生的事件先收集到这里，再按因果顺序处理。
    settlement_events: Option<Vec<Event>>,
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
            settlement_events: None,
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

    /// 批次外追加待处理事件；命令执行期间产生当前事件的子事件。
    /// 子事件先于兄弟事件结算，外部事件之间保持先进先出。
    /// 外部应通过 BeforeTurn 等流程入口发起行动，Turn 由流程驱动生成。
    pub fn queue_event(&mut self, event: Event) {
        if let Some(children) = &mut self.settlement_events {
            children.push(event);
        } else {
            self.event_queue.push_back(event);
        }
    }

    /// 同步结算一个事件及其全部子事件，不消费其他外部事件。
    /// 监听者对根事件的修改会写回参数；流程事件留给 pump 处理。
    ///
    /// 每个事件先完成全部监听者和命令，再按深度优先处理子事件。
    /// 死亡确认排在 BeforePlayerDeath 的子事件之后，终局判断排在整批之后。
    /// 因而死亡触发的反击或召唤可以改变胜负，AfterTurn 不能越过攻击结算。
    ///
    /// # Panics
    /// 在命令中重入驱动会触发断言；请使用 queue_event 产生反应。
    /// 结算过程发生 panic 后，不支持恢复该 World 的执行。
    pub fn apply_event(&mut self, event: &mut Event) {
        self.assert_not_settling();
        if self.is_end() {
            return;
        }

        self.settlement_events = Some(Vec::new());
        let mut pending = VecDeque::from([SettlementWork::Dispatch(event.clone())]);
        let mut settled = Vec::new();
        let mut is_root = true;
        while let Some(work) = pending.pop_front() {
            let mut current = match work {
                SettlementWork::Dispatch(event) => event,
                SettlementWork::ConfirmDeath(id) => {
                    self.resolve_death(id);
                    let children = std::mem::take(self.settlement_events.as_mut().unwrap());
                    for child in children.into_iter().rev() {
                        pending.push_front(SettlementWork::Dispatch(child));
                    }
                    continue;
                }
            };
            if matches!(current, Event::BeforePlayerDeath(id)
                if self.state.get_player(id).is_none_or(|player| player.hp() != 0))
            {
                is_root = false;
                continue;
            }
            self.dispatch_event(&mut current);
            if let Event::BeforePlayerDeath(id) = current {
                pending.push_front(SettlementWork::ConfirmDeath(id));
            }
            if is_root {
                *event = current.clone();
                is_root = false;
            }
            settled.push(current);
            let children = std::mem::take(self.settlement_events.as_mut().unwrap());
            // 反向插入队头，保持兄弟事件顺序，并先完成每个事件的整条反应链。
            for child in children.into_iter().rev() {
                pending.push_front(SettlementWork::Dispatch(child));
            }
        }
        self.settlement_events = None;

        if let Some(result) = self
            .end_conditions
            .iter_mut()
            .find_map(|condition| condition.check(&self.state, event))
        {
            self.apply_end(result);
        }
        if !self.is_end() {
            for settled_event in settled {
                self.event_queue
                    .extend(self.flow.advance(&self.state, &settled_event));
            }
        }
    }

    fn dispatch_event(&mut self, event: &mut Event) {
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
    }

    fn assert_not_settling(&self) {
        assert!(
            self.settlement_events.is_none(),
            "不能在结算期间重入驱动，请使用 queue_event"
        );
    }

    /// BeforePlayerDeath 的整条反应链完成后再确认死亡；通知前立即移除玩家。
    fn resolve_death(&mut self, id: PlayerId) {
        if self.state.get_player(id).is_some_and(|p| p.hp() == 0) {
            self.log(LogEntry::Death { player_id: id });
            self.state.remove_player(id);
            self.queue_event(Event::AfterPlayerDeath(id));
        }
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
        self.assert_not_settling();
        self.apply_event(&mut Event::DuelStart);
        self.pump();
    }

    /// 逐批结算外层队列直到结束或排空；不能在命令中重入调用。
    pub fn pump(&mut self) {
        self.assert_not_settling();
        while !self.is_end() {
            let Some(mut event) = self.event_queue.pop_front() else {
                break;
            };
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
