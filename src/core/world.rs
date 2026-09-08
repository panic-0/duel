use super::{
    buff_data::{BuffData, BuffRecord, DestructionReason},
    event::Event,
    log::{LogEntry, Logger},
    operation::{
        ChangeSet, DestroyedInfo, ErasedOperation, ExecutionContext, HpChange, Operation,
        OperationError, OperationOutcome, OperationResult, SubmissionResult,
    },
    player::Player,
    query::Query,
    state::GameState,
    system::{Fact, NoticeKind, Priority, Subject, System},
    BuffId, PlayerId,
};
use std::any::{Any, TypeId};
use std::collections::{BTreeMap, HashMap};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// 引擎内置的默认回合上限，防止无限对局挂起
pub const MAX_ROUNDS: u32 = 10000;

/// 对局结束的判定结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameResult {
    /// 剩最后一名玩家
    LastStanding,
    /// 达到回合上限，平局
    Draw,
}

/// 反应链的最大同步嵌套深度。子操作队列是迭代的，不受此限制；
/// 只有“操作发布通知 → System 响应 → 再发布通知”的递归路径受它约束。
const MAX_REACTION_DEPTH: usize = 256;

#[derive(Debug)]
struct SystemSlot {
    system: Box<dyn System>,
}

#[derive(Debug, Clone, Copy)]
struct NoticeRoute {
    priority: Priority,
    system_order: usize,
    system_id: usize,
}

#[derive(Debug, Clone, Copy)]
struct RankedCandidate {
    priority: Priority,
    subject_order: usize,
    system_order: usize,
    system_id: usize,
    subject: Subject,
}

/// 一次分发的事实：普通事件或携带暂存数据的销毁事实。
pub(crate) enum Notice {
    Event(Event),
    Destroyed {
        buff_id: BuffId,
        owner: Option<PlayerId>,
        reason: DestructionReason,
        data: Box<dyn Any>,
    },
}

impl Notice {
    fn kind(&self) -> NoticeKind {
        match self {
            Notice::Event(event) => NoticeKind::Event(event.event_type()),
            Notice::Destroyed { .. } => NoticeKind::Destroyed,
        }
    }

    fn fact(&self) -> Fact<'_> {
        match self {
            Notice::Event(event) => Fact::Event(event),
            Notice::Destroyed {
                buff_id,
                owner,
                reason,
                data,
            } => Fact::Destroyed(super::system::Destruction {
                buff_id: *buff_id,
                owner: *owner,
                reason: *reason,
                data: data.as_ref(),
            }),
        }
    }
}

struct StagedDestruction {
    buff_id: BuffId,
    owner: Option<PlayerId>,
    reason: DestructionReason,
    data: Box<dyn Any>,
}

/// 执行器：管理基础状态、数据实例与扩展资源，统一运行 System 响应与
/// Operation，负责候选排序、受控关联提交和事实通知。不实现伤害或死亡玩法。
#[derive(Debug)]
pub struct World {
    state: GameState,
    records: BTreeMap<BuffId, BuffRecord>,
    buff_id_counter: BuffId,
    systems: Vec<SystemSlot>,
    system_id_counter: usize,
    /// 数据实例创建与 System 注册共用的单调顺序来源。
    subject_seq: usize,
    notice_index: HashMap<NoticeKind, Vec<NoticeRoute>>,
    /// 中性扩展存储：按类型存放业务资源（规则注册表、进行中标记等），
    /// 基础层不解释其内容。
    resources: HashMap<TypeId, Box<dyn Any>>,
    end: bool,
    operation_error: Option<OperationError>,
    operation_depth: usize,
}

impl Default for World {
    fn default() -> Self {
        World::new()
    }
}

impl World {
    pub fn new() -> Self {
        World {
            state: GameState::new(),
            records: BTreeMap::new(),
            buff_id_counter: 0,
            systems: Vec::new(),
            system_id_counter: 0,
            subject_seq: 0,
            notice_index: HashMap::new(),
            resources: HashMap::new(),
            end: false,
            operation_error: None,
            operation_depth: 0,
        }
    }

    // —— 初始化装配（与运行期受控修改分开） ——

    pub fn add_player(&mut self, player: Player) -> PlayerId {
        self.state.add_player(player)
    }

    /// 注册一个 System。System 的注册与数据实例生命周期独立：
    /// 删除一份护盾实例不会删除护盾 System，同一个 System 也不要重复注册。
    pub fn add_system(&mut self, system: impl System + 'static) {
        let system_id = self.system_id_counter;
        self.system_id_counter += 1;
        // 独立响应的主体顺序 = System 注册顺序，与数据实例创建共用同一单调来源。
        let order = self.next_subject();
        for (kind, priority) in system.subscriptions() {
            let routes = self.notice_index.entry(kind).or_default();
            routes.push(NoticeRoute {
                priority,
                system_order: order,
                system_id,
            });
            routes.sort_by_key(|route| (route.priority, route.system_order));
        }
        self.systems.push(SystemSlot {
            system: Box::new(system),
        });
    }

    /// 注册一份数据实例，返回稳定身份（BuffId）。
    pub fn add_data<D: BuffData + 'static>(&mut self, owner: Option<PlayerId>, data: D) -> BuffId {
        let id = self.buff_id_counter;
        self.buff_id_counter += 1;
        let subject_order = self.next_subject();
        self.records.insert(
            id,
            BuffRecord {
                owner,
                subject_order,
                data: Box::new(data),
            },
        );
        id
    }

    /// 按身份读取数据实例。
    pub fn get_data<T: BuffData>(&self, id: BuffId) -> Option<&T> {
        let record = self.records.get(&id)?;
        record.data.downcast_ref::<T>()
    }

    pub fn set_logger(&mut self, logger: Logger) {
        self.state.set_logger(logger);
    }

    pub fn log(&self, entry: LogEntry) {
        self.state.log(entry);
    }

    // —— 只读访问 ——

    pub fn get_players(&self) -> &BTreeMap<PlayerId, Player> {
        self.state.get_players()
    }

    pub fn get_player(&self, id: PlayerId) -> Option<&Player> {
        self.state.get_player(id)
    }

    /// 仅限初始化配置；运行期生命变化必须走受控提交。
    pub fn get_player_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        self.state.get_player_mut(id)
    }

    pub(crate) fn state_view(&self) -> &GameState {
        &self.state
    }

    /// 只读查询视图。
    pub fn query(&self) -> Query<'_> {
        Query {
            state: &self.state,
            records: &self.records,
            resources: &self.resources,
        }
    }

    pub fn is_end(&self) -> bool {
        self.end
    }

    pub fn is_operation_failed(&self) -> bool {
        self.operation_error.is_some()
    }

    // —— 中性扩展存储 ——

    pub(crate) fn set_resource<T: Any>(&mut self, value: T) {
        self.resources.insert(TypeId::of::<T>(), Box::new(value));
    }

    pub(crate) fn resource<T: Any>(&self) -> Option<&T> {
        self.resources
            .get(&TypeId::of::<T>())
            .and_then(|value| value.downcast_ref::<T>())
    }

    pub(crate) fn resource_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.resources
            .get_mut(&TypeId::of::<T>())
            .and_then(|value| value.downcast_mut::<T>())
    }

    pub(crate) fn resource_mut_or_insert_with<T: Any>(
        &mut self,
        default: impl FnOnce() -> T,
    ) -> &mut T {
        self.resources
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(default()))
            .downcast_mut::<T>()
            .expect("扩展资源类型与键一致")
    }

    // —— 根入口与执行 ——

    /// 运行一个根 Operation。正式终局后的新根请求被无副作用拒绝，
    /// 不把正常终局记为执行失败；不自动装配默认规则。
    pub fn execute<O: Operation>(
        &mut self,
        operation: O,
    ) -> Result<OperationOutcome, OperationError> {
        if self.end {
            return Err(OperationError::Invalid("对局已结束".into()));
        }
        self.check_operation_failed()?;
        let result = self.execute_erased(Box::new(operation));
        if let Err(error) = &result {
            self.record_operation_error(error.clone());
        }
        result
    }

    fn execute_erased(
        &mut self,
        operation: Box<dyn ErasedOperation>,
    ) -> Result<OperationOutcome, OperationError> {
        // 进入任何操作前先拦截已有失败与终局；终局属正常状态，按跳过处理。
        self.check_operation_failed()?;
        if self.end {
            return Ok((OperationResult::Skipped, None));
        }
        if self.operation_depth >= MAX_REACTION_DEPTH {
            return Err(OperationError::Failed(format!(
                "反应链嵌套深度超过上限 {MAX_REACTION_DEPTH}"
            )));
        }
        self.operation_depth += 1;
        let outcome = (|| {
            let mut stack: Vec<Box<dyn ErasedOperation>> = vec![operation];
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
        operation: Box<dyn ErasedOperation>,
    ) -> Result<OperationOutcome, OperationError> {
        let result = self.execute_erased(operation);
        if let Err(error) = &result {
            self.record_operation_error(error.clone());
        }
        result
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
    pub(crate) fn check_operation_failed(&self) -> Result<(), OperationError> {
        match &self.operation_error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    pub(crate) fn record_operation_error(&mut self, error: OperationError) {
        if self.operation_error.is_none() {
            self.operation_error = Some(error);
        }
    }

    // —— 受控提交与通知 ——

    /// 中性关联提交：全部关联变化写入完成后，才按
    /// “基础状态事实（提交声明顺序）→ 销毁事实（BuffId 升序）”开放通知；
    /// 每条事实的嵌套反应完整结束后，再继续本组下一条事实。
    pub(crate) fn submit(
        &mut self,
        changes: ChangeSet,
    ) -> Result<SubmissionResult, OperationError> {
        self.check_operation_failed()?;
        let ChangeSet {
            hp,
            remove_player,
            destroy,
        } = changes;
        let mut result = SubmissionResult::default();
        let mut staged: Vec<StagedDestruction> = Vec::new();

        // —— 关联写入阶段：不运行任何玩法响应 ——
        if let Some(request) = hp {
            result.hp = match request {
                super::operation::HpRequest::Damage(target_id, amount) => {
                    self.commit_damage(target_id, amount)
                }
                super::operation::HpRequest::Heal(target_id, amount) => {
                    self.commit_heal(target_id, amount)
                }
            };
        }
        if let Some(player_id) = remove_player {
            result.player_removed = self.take_player(player_id).is_some();
            if result.player_removed {
                let owned: Vec<BuffId> = self
                    .records
                    .iter()
                    .filter(|(_, record)| record.owner == Some(player_id))
                    .map(|(id, _)| *id)
                    .collect();
                for id in owned {
                    if let Some(record) = self.records.remove(&id) {
                        staged.push(StagedDestruction {
                            buff_id: id,
                            owner: record.owner,
                            reason: DestructionReason::OwnerDeath,
                            data: record.data,
                        });
                    }
                }
            }
        }
        for (id, reason) in destroy {
            // 同一实例只产生一次实际销毁事实。
            if staged.iter().any(|item| item.buff_id == id) {
                continue;
            }
            if let Some(record) = self.records.remove(&id) {
                staged.push(StagedDestruction {
                    buff_id: id,
                    owner: record.owner,
                    reason,
                    data: record.data,
                });
            }
        }
        staged.sort_by_key(|item| item.buff_id);
        result.destroyed = staged
            .iter()
            .map(|item| DestroyedInfo {
                buff_id: item.buff_id,
                owner: item.owner,
                reason: item.reason,
            })
            .collect();

        // —— 通知阶段：基础状态事实 → 销毁事实 ——
        if let Some(change) = result.hp {
            if change.old_hp != change.new_hp {
                self.settle_and_record(Event::HpChanged {
                    target_id: change.target_id,
                    old_hp: change.old_hp,
                    new_hp: change.new_hp,
                })?;
            }
        }
        for item in staged {
            self.dispatch_notice(Notice::Destroyed {
                buff_id: item.buff_id,
                owner: item.owner,
                reason: item.reason,
                data: item.data,
            })?;
        }
        Ok(result)
    }

    pub(crate) fn settle_operation_event(&mut self, event: Event) -> Result<(), OperationError> {
        // 发布通知是运行期受控入口：世界已记录失败时直接拒绝。
        self.check_operation_failed()?;
        self.dispatch_notice(Notice::Event(event))
    }

    /// 分发一条事实：开始时统一收集候选并按
    /// `(Priority, 响应主体稳定顺序, System 注册顺序)` 升序固定，
    /// 然后逐候选响应；每个候选返回的 Operation 及其反应完整结束后再继续。
    fn dispatch_notice(&mut self, notice: Notice) -> Result<(), OperationError> {
        if self.end {
            // 正式终局后不再启动后续玩法监听者。
            return Ok(());
        }
        let routes = self
            .notice_index
            .get(&notice.kind())
            .cloned()
            .unwrap_or_default();
        let mut candidates: Vec<RankedCandidate> = Vec::new();
        {
            let fact = notice.fact();
            let query = self.query();
            for route in &routes {
                let slot = &self.systems[route.system_id];
                for subject in slot.system.candidates(&fact, &query) {
                    let subject_order = match subject {
                        Subject::Instance(id) => query.subject_order(id).unwrap_or(0),
                        Subject::Standalone => route.system_order,
                    };
                    candidates.push(RankedCandidate {
                        priority: route.priority,
                        subject_order,
                        system_order: route.system_order,
                        system_id: route.system_id,
                        subject,
                    });
                }
            }
        }
        candidates.sort_by(|a, b| {
            (a.priority, a.subject_order, a.system_order).cmp(&(
                b.priority,
                b.subject_order,
                b.system_order,
            ))
        });

        for candidate in candidates {
            if self.is_end() {
                break;
            }
            // C3A：轮到候选时检查实例仍存在；新增实例不补收当前通知。
            if let Subject::Instance(id) = candidate.subject {
                if !self.records.contains_key(&id) {
                    continue;
                }
            }
            let outcome = {
                let fact = notice.fact();
                let query = self.query();
                self.systems[candidate.system_id]
                    .system
                    .respond(&fact, candidate.subject, &query)
            };
            let ops = match outcome {
                Ok(ops) => ops,
                Err(error) => {
                    self.record_operation_error(error.clone());
                    return Err(error);
                }
            };
            // D4：候选返回的 Operation 已产生，即使后续反应销毁了来源数据也继续执行。
            for operation in ops {
                if self.is_end() {
                    break;
                }
                self.execute_inline(operation)?;
            }
        }
        Ok(())
    }

    /// 结算事件；错误在记录后原样返回，保证失败状态对后续受控入口可见。
    fn settle_and_record(&mut self, event: Event) -> Result<(), OperationError> {
        let result = self.dispatch_notice(Notice::Event(event));
        if let Err(error) = &result {
            self.record_operation_error(error.clone());
        }
        result
    }

    // —— 中性生命变化（提交内部使用） ——

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

    // —— 其余受控入口 ——

    pub(crate) fn take_player(&mut self, id: PlayerId) -> Option<Player> {
        self.state.remove_player(id)
    }

    pub(crate) fn end_game(&mut self, result: GameResult) -> Result<(), OperationError> {
        self.check_operation_failed()?;
        if self.end {
            return Ok(());
        }
        if matches!(result, GameResult::Draw) {
            self.log(LogEntry::Draw);
        }
        self.end = true;
        Ok(())
    }

    fn next_subject(&mut self) -> usize {
        self.subject_seq += 1;
        self.subject_seq
    }

    // —— 注册表诊断 ——

    pub fn get_event_registration_count(&self, kind: NoticeKind) -> usize {
        self.notice_index
            .get(&kind)
            .map(|routes| routes.len())
            .unwrap_or(0)
    }

    pub fn get_registry_stats(&self) -> (usize, usize, usize) {
        let total_kinds = self.notice_index.len();
        let total_routes: usize = self.notice_index.values().map(|routes| routes.len()).sum();
        (total_kinds, total_routes, self.systems.len())
    }

    pub fn validate_registry_consistency(&self) -> bool {
        self.notice_index
            .values()
            .flatten()
            .all(|route| route.system_id < self.systems.len())
    }
}
