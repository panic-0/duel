use super::{
    buff::{Buff, DefaultDeathRule, DefaultVictoryRule, Priority},
    command::Commands,
    event::{Event, EventType},
    flow::{EndCondition, FlowDriver, GameResult, MAX_ROUNDS},
    log::{LogEntry, Logger},
    operation::{ExecutionContext, HpChange, Operation, OperationError, OperationOutcome},
    player::Player,
    state::GameState,
    BuffId, PlayerId,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// 反应链的最大同步嵌套深度。子操作队列是迭代的，不受此限制；
/// 只有“操作发布事件 → Buff 产生操作 → 再发布事件”的递归路径受它约束。
const MAX_REACTION_DEPTH: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EventRegistration {
    priority: Priority,
    buff_id: BuffId,
}

#[derive(Debug)]
struct DispatchFrame {
    event: Event,
    registrations: Vec<EventRegistration>,
    next: usize,
    order: usize,
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

/// Operation 执行器：负责任务栈、受控提交、按订阅分发和阶段推进。
/// 旧 FlowDriver/EndCondition 仅保留为迁移适配。
#[derive(Debug)]
pub struct World {
    state: GameState,
    buffs: BTreeMap<BuffId, Box<dyn Buff>>,
    buff_id_counter: BuffId,
    event_registry: HashMap<EventType, BTreeSet<EventRegistration>>,
    event_queue: VecDeque<Event>,
    /// Some 表示正在结算；命令产生的事件先收集到这里，再按因果顺序处理。
    settlement_events: Option<Vec<Event>>,
    flow: Option<Box<dyn FlowDriver>>,
    end_conditions: Vec<Box<dyn EndCondition>>,
    end: bool,
    operation_error: Option<OperationError>,
    operation_depth: usize,
    default_rules_installed: bool,
    /// 当前活动批次的事件语义：apply_event 旧入口置为 true 并贯穿全部嵌套结算
    /// （含其中 Operation 发布的事件）；Operation 根入口为 false，嵌套继承。
    legacy_dispatch: bool,
    /// 正在进入死亡前流程的角色；同一次死亡不重复进入。
    pub(crate) pending_deaths: Vec<PlayerId>,
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
            flow: None,
            end_conditions: Vec::new(),
            end: false,
            operation_error: None,
            operation_depth: 0,
            default_rules_installed: false,
            legacy_dispatch: false,
            pending_deaths: Vec::new(),
        }
    }

    /// 已废弃：仅影响旧 apply_event 批次入口，不再被 `run()` 使用。
    /// 自定义流程应实现为普通 Operation 并通过 `execute` 组合。
    #[doc(hidden)]
    pub fn set_flow(&mut self, flow: Box<dyn FlowDriver>) {
        self.flow = Some(flow);
    }

    /// 已废弃：仅影响旧 apply_event 批次入口，不再被 `run()` 使用。
    /// 对局轮数上限请使用 `run_with_max_rounds`；自定义胜负请装配检查点 Buff。
    #[doc(hidden)]
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

    pub(crate) fn state_view(&self) -> &GameState {
        &self.state
    }

    /// 运行一个根 Operation。不自动装配默认规则；
    /// 需要默认死亡/胜负规则请先调用 [`World::install_default_rules`]，或使用 `run*`。
    pub fn execute<O: Operation>(
        &mut self,
        operation: O,
    ) -> Result<OperationOutcome, OperationError> {
        self.assert_not_settling();
        if let Some(error) = &self.operation_error {
            return Err(error.clone());
        }
        let result = self.execute_erased(Box::new(operation));
        if let Err(error) = &result {
            self.record_operation_error(error.clone());
        }
        result
    }

    fn execute_erased(
        &mut self,
        operation: Box<dyn super::operation::ErasedOperation>,
    ) -> Result<OperationOutcome, OperationError> {
        // 进入任何操作前先拦截已记录的失败，而不是等它运行之后再检查。
        self.check_operation_failed()?;
        if self.operation_depth >= MAX_REACTION_DEPTH {
            return Err(OperationError::Failed(format!(
                "反应链嵌套深度超过上限 {MAX_REACTION_DEPTH}"
            )));
        }
        self.operation_depth += 1;
        let outcome = (|| {
            let mut stack: Vec<Box<dyn super::operation::ErasedOperation>> = vec![operation];
            let mut root_result = None;
            while let Some(op) = stack.pop() {
                let result = {
                    let mut context = ExecutionContext::new(self);
                    let result = catch_unwind(AssertUnwindSafe(|| op.execute_erased(&mut context)))
                        .map_err(|_| OperationError::Failed("Operation panic，World 已停止".into()))
                        .and_then(|value| value);
                    let children = context.take_children();
                    drop(context);
                    for child in children.into_iter().rev() {
                        stack.push(child);
                    }
                    result
                };
                let value = result?;
                if let Some(error) = &self.operation_error {
                    return Err(error.clone());
                }
                if root_result.is_none() {
                    root_result = Some(value);
                }
            }
            Ok(root_result.expect("Operation 栈不应为空"))
        })();
        self.operation_depth -= 1;
        outcome
    }

    pub(crate) fn execute_child(
        &mut self,
        operation: Box<dyn super::operation::ErasedOperation>,
    ) -> Result<OperationOutcome, OperationError> {
        let result = self.execute_erased(operation);
        if let Err(error) = &result {
            self.record_operation_error(error.clone());
        }
        result
    }

    pub(crate) fn modify_hp(&mut self, target_id: PlayerId, modifier: i64) -> Option<HpChange> {
        let change = self.commit_signed_hp(target_id, modifier)?;
        let HpChange { old_hp, new_hp, .. } = change;
        if old_hp != new_hp {
            if self.default_rules_installed && self.operation_depth > 0 {
                if let Err(error) = self.publish_hp_change(change) {
                    self.record_operation_error(error);
                }
            } else {
                self.queue_event(Event::HpChanged {
                    target_id,
                    old_hp,
                    new_hp,
                });
            }
        }
        if !self.default_rules_installed && old_hp > 0 && new_hp == 0 {
            self.queue_event(Event::BeforePlayerDeath(target_id));
        }
        Some(change)
    }

    pub(crate) fn modify_hp_from_operation(
        &mut self,
        target_id: PlayerId,
        modifier: i64,
    ) -> Result<Option<HpChange>, OperationError> {
        self.check_operation_failed()?;
        let Some(change) = self.commit_signed_hp(target_id, modifier) else {
            return Ok(None);
        };
        self.publish_hp_change(change)?;
        Ok(Some(change))
    }

    fn commit_signed_hp(&mut self, target_id: PlayerId, modifier: i64) -> Option<HpChange> {
        if modifier < 0 {
            self.commit_damage(target_id, modifier.unsigned_abs())
        } else {
            self.commit_heal(target_id, modifier as u64)
        }
    }

    fn commit_damage(&mut self, target_id: PlayerId, amount: u64) -> Option<HpChange> {
        let (old_hp, new_hp, max_hp) = {
            let target = self.state.get_player_mut(target_id)?;
            let old_hp = target.hp();
            let new_hp = target.damage(amount);
            (old_hp, new_hp, target.max_hp())
        };
        self.log(LogEntry::Damage {
            target_id,
            amount,
            hp: new_hp,
            max_hp,
        });
        Some(HpChange {
            target_id,
            old_hp,
            new_hp,
            max_hp,
            requested: if amount > i64::MAX as u64 {
                i64::MIN
            } else {
                -(amount as i64)
            },
            submitted_amount: amount,
        })
    }

    fn commit_heal(&mut self, target_id: PlayerId, amount: u64) -> Option<HpChange> {
        let (old_hp, new_hp, max_hp) = {
            let target = self.state.get_player_mut(target_id)?;
            let old_hp = target.hp();
            let new_hp = target.heal(amount);
            (old_hp, new_hp, target.max_hp())
        };
        self.log(LogEntry::Heal {
            target_id,
            amount,
            hp: new_hp,
            max_hp,
        });
        Some(HpChange {
            target_id,
            old_hp,
            new_hp,
            max_hp,
            requested: amount.min(i64::MAX as u64) as i64,
            submitted_amount: amount,
        })
    }

    fn publish_hp_change(&mut self, change: HpChange) -> Result<(), OperationError> {
        if change.old_hp == change.new_hp {
            return Ok(());
        }
        self.settle_and_record(Event::HpChanged {
            target_id: change.target_id,
            old_hp: change.old_hp,
            new_hp: change.new_hp,
        })
    }

    pub(crate) fn submit_damage(
        &mut self,
        mut damage: super::operation::DamageContext,
    ) -> Result<Option<HpChange>, OperationError> {
        self.check_operation_failed()?;
        // 参数修改窗口：按统一优先级和注册顺序调整草稿，前一个修改对后一个可见。
        let mut modifiers: Vec<(Priority, BuffId)> = self
            .buffs
            .iter()
            .filter_map(|(id, buff)| buff.damage_modification().map(|priority| (priority, *id)))
            .collect();
        modifiers.sort();
        for (_, buff_id) in modifiers {
            if let Some(buff) = self.buffs.get(&buff_id) {
                buff.modify_damage(&mut damage, &self.state, buff_id);
            }
        }
        let Some(change) = self.commit_damage(damage.target_id, damage.amount) else {
            return Ok(None);
        };
        // 与伤害一起受控提交的消耗（如一次性护盾）在通知反应前处理。
        for buff_id in damage.take_consumption() {
            self.remove_buff(buff_id);
        }
        self.publish_hp_change(change)?;
        Ok(Some(change))
    }

    pub(crate) fn submit_heal(
        &mut self,
        target_id: PlayerId,
        amount: u64,
    ) -> Result<Option<HpChange>, OperationError> {
        self.check_operation_failed()?;
        let Some(change) = self.commit_heal(target_id, amount) else {
            return Ok(None);
        };
        self.publish_hp_change(change)?;
        Ok(Some(change))
    }

    fn execute_inline(
        &mut self,
        operation: Box<dyn Operation>,
    ) -> Result<OperationOutcome, OperationError> {
        let result = self.execute_erased(Box::new(operation));
        if let Err(error) = &result {
            self.record_operation_error(error.clone());
        }
        result
    }

    /// 世界已记录失败时返回该错误；受控入口在执行任何新变化前检查。
    fn check_operation_failed(&self) -> Result<(), OperationError> {
        match &self.operation_error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    /// 结算事件；错误在记录后原样返回，保证失败状态对后续受控入口可见。
    fn settle_and_record(&mut self, event: Event) -> Result<(), OperationError> {
        let result = self.settle_operation_event(event);
        if let Err(error) = &result {
            self.record_operation_error(error.clone());
        }
        result
    }

    pub(crate) fn settle_operation_event(&mut self, event: Event) -> Result<(), OperationError> {
        self.dispatch_event(event, self.legacy_dispatch).map(|_| ())
    }

    /// 按当前状态收集该玩家在 `after` 之后的下一个正常行动。
    /// 返回值携带稳定技能槽位 `(buff_id, 集合内下标)` 作为游标，
    /// 技能可用性变化不会让调用方错位；每次查询都重新判断资格。
    pub(crate) fn normal_action_after(
        &mut self,
        player_id: PlayerId,
        after: Option<(BuffId, usize)>,
    ) -> Option<((BuffId, usize), Box<dyn Operation>)> {
        let mut next: Option<((BuffId, usize), Box<dyn Operation>)> = None;
        for (buff_id, buff) in self.buffs.iter_mut() {
            if buff.owner() != Some(player_id) {
                continue;
            }
            let Some(set) = buff.ability_set() else {
                continue;
            };
            for (slot, action) in set.normal_actions(player_id, &self.state) {
                let key = (*buff_id, slot);
                if after.is_some_and(|previous| key <= previous) {
                    continue;
                }
                if next.as_ref().is_none_or(|(best_key, _)| key < *best_key) {
                    next = Some((key, action));
                }
            }
        }
        next
    }

    pub(crate) fn record_operation_error(&mut self, error: OperationError) {
        if self.operation_error.is_none() {
            self.operation_error = Some(error);
        }
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

    /// 受控访问：只能就地修改 buff 自身状态，不能替换订阅对象，
    /// 因此不会绕过事件注册索引。
    pub fn get_buff_mut(&mut self, id: BuffId) -> Option<&mut dyn Buff> {
        match self.buffs.get_mut(&id) {
            Some(buff) => Some(buff.as_mut()),
            None => None,
        }
    }

    pub fn set_logger(&mut self, logger: Logger) {
        self.state.set_logger(logger);
    }

    pub fn log(&self, entry: LogEntry) {
        self.state.log(entry);
    }

    /// 迁移适配：Operation 应使用 ExecutionContext::publish。
    #[doc(hidden)]
    /// 子事件先于兄弟事件结算，外部事件之间保持先进先出。
    /// 外部应通过 BeforeTurn 等流程入口发起行动，Turn 由流程驱动生成。
    pub fn queue_event(&mut self, event: Event) {
        if let Some(children) = &mut self.settlement_events {
            children.push(event);
        } else {
            self.event_queue.push_back(event);
        }
    }

    /// 迁移适配：Operation 应通过 ExecutionContext::settle。
    #[doc(hidden)]
    /// 旧事件入口仅供迁移适配，新的规则应使用 Operation 上下文。
    ///
    /// 每个事件先完成全部监听者和命令，再按深度优先处理子事件。
    /// 死亡确认排在 BeforePlayerDeath 的子事件之后，终局判断排在整批之后。
    /// 因而死亡触发的反击或召唤可以改变胜负，AfterTurn 不能越过攻击结算。
    ///
    /// # Panics
    /// 在结算期间重入驱动会触发断言；请使用受控上下文产生反应。
    /// 结算过程发生 panic 后，不支持恢复该 World 的执行。
    #[doc(hidden)]
    pub fn apply_event(&mut self, event: &mut Event) {
        self.assert_not_settling();
        if self.is_end() {
            return;
        }

        self.legacy_dispatch = true;
        self.settlement_events = Some(Vec::new());
        let settled = match self.dispatch_event(event.clone(), true) {
            Ok(settled) => settled,
            Err(error) => {
                self.settlement_events = None;
                self.legacy_dispatch = false;
                self.record_operation_error(error);
                return;
            }
        };
        if let Some(root) = settled.first() {
            *event = root.clone();
        }
        self.settlement_events = None;
        self.legacy_dispatch = false;

        if let Some(result) = self
            .end_conditions
            .iter_mut()
            .find_map(|condition| condition.check(&self.state, event))
        {
            self.apply_end(result);
        }
        if !self.is_end() {
            for settled_event in settled {
                let next = if let Some(flow) = self.flow.as_mut() {
                    flow.advance(&self.state, &settled_event)
                } else {
                    Self::default_flow(&self.state, &settled_event)
                };
                self.event_queue.extend(next);
            }
        }
    }

    fn default_flow(state: &GameState, event: &Event) -> Vec<Event> {
        use std::ops::Bound::{Excluded, Unbounded};
        let next_turn = |round: u32, player_id: PlayerId| {
            state
                .get_players()
                .range((Excluded(player_id), Unbounded))
                .find(|(_, player)| player.is_alive())
                .map(|(&id, _)| Event::BeforeTurn {
                    round,
                    player_id: id,
                })
                .unwrap_or(Event::RoundEnd { round })
        };
        match event {
            Event::DuelStart => vec![Event::RoundStart { round: 1 }],
            Event::RoundStart { round } => state
                .get_players()
                .iter()
                .find(|(_, player)| player.is_alive())
                .map(|(id, _)| Event::BeforeTurn {
                    round: *round,
                    player_id: *id,
                })
                .into_iter()
                .collect(),
            Event::BeforeTurn { round, player_id }
                if !state
                    .get_player(*player_id)
                    .is_some_and(|player| player.is_alive()) =>
            {
                vec![next_turn(*round, *player_id)]
            }
            Event::BeforeTurn { round, player_id } => vec![Event::Turn {
                round: *round,
                player_id: *player_id,
            }],
            Event::Turn { round, player_id } => vec![Event::AfterTurn {
                round: *round,
                player_id: *player_id,
            }],
            Event::AfterTurn { round, player_id } => vec![next_turn(*round, *player_id)],
            Event::RoundEnd { round } => vec![Event::RoundStart { round: *round + 1 }],
            _ => Vec::new(),
        }
    }

    /// `legacy` 标记旧 apply_event 入口：只有它保留可变事件回调与 Command 语义，
    /// Operation 路径把事件当作不可变事实，监听者只能产生后续操作。
    fn dispatch_event(&mut self, event: Event, legacy: bool) -> Result<Vec<Event>, OperationError> {
        if !self.event_is_applicable(&event) {
            return Ok(Vec::new());
        }
        let mut settled = Vec::new();
        let registrations = self
            .event_registry
            .get(&event.event_type())
            .map(|items| items.iter().copied().collect())
            .unwrap_or_default();
        let mut frames = vec![DispatchFrame {
            event,
            registrations,
            next: 0,
            order: 0,
        }];
        let mut next_order = 1;

        while !frames.is_empty() {
            let frame_index = frames.len() - 1;
            if frames[frame_index].next >= frames[frame_index].registrations.len() {
                let completed = frames[frame_index].event.clone();
                let order = frames[frame_index].order;
                if let Event::BeforePlayerDeath(player_id) = completed {
                    if !self.default_rules_installed {
                        self.resolve_legacy_death(player_id);
                    }
                }
                if let Event::AfterPlayerDeath(player_id) = completed {
                    if !self.default_rules_installed {
                        self.cleanup_owned_buffs(player_id);
                    }
                }
                frames.pop();
                if settled.len() <= order {
                    settled.resize(order + 1, Event::DuelStart);
                }
                settled[order] = completed;
                let children = self
                    .settlement_events
                    .as_mut()
                    .map(std::mem::take)
                    .unwrap_or_default();
                for child in children.into_iter().rev() {
                    let registrations = self
                        .event_registry
                        .get(&child.event_type())
                        .map(|items| items.iter().copied().collect())
                        .unwrap_or_default();
                    frames.push(DispatchFrame {
                        event: child,
                        registrations,
                        next: 0,
                        order: next_order,
                    });
                    next_order += 1;
                }
                continue;
            }

            let registration = frames[frame_index].registrations[frames[frame_index].next];
            frames[frame_index].next += 1;
            let event = &frames[frame_index].event;
            if !self.event_is_applicable(event) {
                frames[frame_index].next = frames[frame_index].registrations.len();
                continue;
            }
            let Some(buff) = self.buffs.get(&registration.buff_id) else {
                continue;
            };
            if !Self::owner_matches_event(buff.as_ref(), event) {
                continue;
            }

            let mut operations = Vec::new();
            let mut commands = Commands::default();
            if let Some(buff) = self.buffs.get_mut(&registration.buff_id) {
                operations = buff.operations(
                    &frames[frame_index].event,
                    &self.state,
                    registration.buff_id,
                );
                if legacy {
                    buff.on_event(
                        &mut frames[frame_index].event,
                        &self.state,
                        &mut commands,
                        registration.buff_id,
                    );
                }
            }
            for operation in operations {
                if self.is_end() {
                    break;
                }
                self.execute_inline(operation)?;
                if let Some(error) = &self.operation_error {
                    return Err(error.clone());
                }
            }
            if legacy {
                for command in commands.commands {
                    command.apply(self);
                }
            }

            let children = self
                .settlement_events
                .as_mut()
                .map(std::mem::take)
                .unwrap_or_default();
            for child in children.into_iter().rev() {
                let registrations = self
                    .event_registry
                    .get(&child.event_type())
                    .map(|items| items.iter().copied().collect())
                    .unwrap_or_default();
                frames.push(DispatchFrame {
                    event: child,
                    registrations,
                    next: 0,
                    order: next_order,
                });
                next_order += 1;
            }
        }
        Ok(settled)
    }

    /// 旧 apply_event 语义：死亡前事件按零血事实过滤（迁移适配保留）。
    /// 新 Operation 路径不做此特判——事实是否仍适用由 DeathOperation
    /// 与各监听者按当前状态自行判断。
    fn event_is_applicable(&self, event: &Event) -> bool {
        if !self.legacy_dispatch {
            return true;
        }
        !matches!(event, Event::BeforePlayerDeath(id)
            if self.state.get_player(*id).is_none_or(|player| player.hp() != 0))
    }

    fn owner_matches_event(buff: &dyn Buff, event: &Event) -> bool {
        let Some(owner) = buff.owner() else {
            return true;
        };
        match *event {
            Event::HpChanged { target_id, .. }
            | Event::BeforePlayerDeath(target_id)
            | Event::AfterPlayerDeath(target_id) => owner == target_id,
            Event::BeforeTurn { player_id, .. }
            | Event::Turn { player_id, .. }
            | Event::AfterTurn { player_id, .. } => owner == player_id,
            Event::BeforePlayerAttack {
                source_id,
                target_id,
            }
            | Event::PlayerAttack {
                source_id,
                target_id,
                ..
            }
            | Event::AfterPlayerAttack {
                source_id,
                target_id,
                ..
            } => owner == source_id || owner == target_id,
            _ => true,
        }
    }

    fn assert_not_settling(&self) {
        assert!(
            self.settlement_events.is_none(),
            "不能在结算期间重入驱动，请使用 queue_event"
        );
    }

    pub(crate) fn cleanup_owned_buffs(&mut self, player_id: PlayerId) {
        let ids: Vec<_> = self
            .buffs
            .iter()
            .filter_map(|(id, buff)| (buff.owner() == Some(player_id)).then_some(*id))
            .collect();
        for id in ids {
            self.remove_buff(id);
        }
    }

    fn resolve_legacy_death(&mut self, player_id: PlayerId) {
        if self
            .state
            .get_player(player_id)
            .is_none_or(|player| player.hp() != 0)
        {
            return;
        }
        self.log(LogEntry::Death { player_id });
        self.state.remove_player(player_id);
        self.queue_event(Event::AfterPlayerDeath(player_id));
    }

    fn apply_end(&mut self, result: GameResult) {
        if matches!(result, GameResult::Draw) {
            self.log(LogEntry::Draw);
        }
        self.end = true;
    }

    pub(crate) fn end_game(&mut self, result: GameResult) {
        self.apply_end(result);
    }

    pub fn is_end(&self) -> bool {
        self.end
    }

    pub fn is_operation_failed(&self) -> bool {
        self.operation_error.is_some()
    }

    /// 显式装配默认规则：零血死亡判断与最后一人生还的胜负检查点 Buff。
    /// `run*` 自动装配；直接 `execute` 的调用方按需自行装配，也可只装配自己的规则。
    pub fn install_default_rules(&mut self) {
        if self.default_rules_installed {
            return;
        }
        self.default_rules_installed = true;
        self.add_buff(Box::new(DefaultDeathRule));
        self.add_buff(Box::new(DefaultVictoryRule));
    }
}

impl World {
    /// 完整跑一局：装配默认规则并执行 DuelOperation，返回引擎错误（如反应链失败）。
    /// 自定义流程请直接组合 RoundOperation/TurnOperation 并调用 `execute`；
    /// 旧 set_flow / add_end_condition 不再影响本方法。
    pub fn run(&mut self) -> Result<(), OperationError> {
        self.install_default_rules();
        self.execute(super::operation::DuelOperation::new(MAX_ROUNDS))
            .map(|_| ())
    }

    pub fn run_with_max_rounds(&mut self, max_rounds: u32) -> Result<(), OperationError> {
        self.install_default_rules();
        self.execute(super::operation::DuelOperation::new(max_rounds))
            .map(|_| ())
    }

    /// 逐批结算外层队列直到结束或排空；不能在命令中重入调用。
    #[doc(hidden)]
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
