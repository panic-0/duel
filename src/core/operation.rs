use std::any::Any;
use std::collections::VecDeque;
use std::fmt::Debug;

use super::buff_data::{BuffData, DestructionReason};
use super::query::Query;
use super::world::World;
use super::{event::Event, state::GameState, BuffId, PlayerId};

/// 操作返回的值。在执行器边界做类型擦除，让互不相关的操作共用同一个迭代工作栈。
pub type OperationValue = Box<dyn Any>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationError {
    Invalid(String),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationResult {
    Completed,
    Skipped,
}

pub type OperationOutcome = (OperationResult, Option<OperationValue>);

pub fn completed() -> Result<OperationOutcome, OperationError> {
    Ok((OperationResult::Completed, None))
}

pub fn completed_with<T: Any>(value: T) -> Result<OperationOutcome, OperationError> {
    Ok((OperationResult::Completed, Some(Box::new(value))))
}

pub fn skipped() -> Result<OperationOutcome, OperationError> {
    Ok((OperationResult::Skipped, None))
}

/// 一段游戏行为。操作刻意保持小巧：复杂规则自己持有阶段状态，
/// 通过执行上下文调度子操作，而不是让执行器了解这条规则。
pub trait Operation: Debug + 'static {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError>;
}

impl Operation for Box<dyn Operation> {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        (*self).execute(context)
    }
}

pub(crate) trait ErasedOperation: Debug {
    fn execute_erased(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError>;
}

impl<T: Operation> ErasedOperation for T {
    fn execute_erased(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        self.execute(context)
    }
}

/// 交给操作的受限能力集合。世界本身不暴露，
/// 因此操作无法任意修改状态或重入驱动器。
pub struct ExecutionContext<'a> {
    world: &'a mut World,
    children: VecDeque<Box<dyn ErasedOperation>>,
}

pub type OperationContext<'a> = ExecutionContext<'a>;

impl<'a> ExecutionContext<'a> {
    pub(crate) fn new(world: &'a mut World) -> Self {
        Self {
            world,
            children: VecDeque::new(),
        }
    }

    pub fn state(&self) -> &GameState {
        self.world.state_view()
    }

    /// 只读查询视图：玩家状态、数据实例与扩展资源。
    pub fn query(&self) -> Query<'_> {
        self.world.query()
    }

    /// 发布事件并等待其全部响应完成。错误记入世界后本调用继续返回；
    /// 后续受控入口（提交、子操作）会拒绝执行，根调用最终返回该错误。
    pub fn publish(&mut self, event: Event) {
        if let Err(error) = self.try_publish(event) {
            self.world.record_operation_error(error);
        }
    }

    /// 发布事件并等待其全部响应完成；事件是不可变事实，没有读回值。
    pub fn try_publish(&mut self, event: Event) -> Result<(), OperationError> {
        self.world.settle_operation_event(event)
    }

    /// 同步执行子操作；返回时，子操作及其事件响应均已完成。
    pub fn execute<O: Operation>(
        &mut self,
        operation: O,
    ) -> Result<OperationOutcome, OperationError> {
        self.world.execute_child(Box::new(operation))
    }

    /// 同步执行类型擦除后的子操作。
    pub fn execute_boxed(
        &mut self,
        operation: Box<dyn Operation>,
    ) -> Result<OperationOutcome, OperationError> {
        self.world.execute_child(Box::new(operation))
    }

    pub fn log(&self, entry: super::log::LogEntry) {
        self.world.log(entry);
    }

    pub fn is_end(&self) -> bool {
        self.world.is_end()
    }

    /// 结束对局。已终局时重复调用为无副作用成功。
    pub fn end_game(&mut self, result: super::world::GameResult) -> Result<(), OperationError> {
        self.world.end_game(result)
    }

    /// 中性关联提交：本组变化全部写入后才按约定顺序开放通知。
    pub fn submit(&mut self, changes: ChangeSet) -> Result<SubmissionResult, OperationError> {
        self.world.submit(changes)
    }

    /// 提交一次生命修改（单条中性变化）并完成其事件响应。
    /// `Ok(None)` 表示目标不存在（可解释的跳过）；`Err` 表示响应链失败且已记录。
    pub fn modify_hp(
        &mut self,
        target_id: PlayerId,
        modifier: i64,
    ) -> Result<Option<HpChange>, OperationError> {
        let changes = ChangeSet::new().hp(target_id, modifier);
        Ok(self.world.submit(changes)?.hp)
    }
    /// 注册一份数据实例，返回稳定身份；销毁事实由后续提交产生。
    pub fn add_data(
        &mut self,
        owner: Option<PlayerId>,
        data: impl BuffData + 'static,
    ) -> Result<BuffId, OperationError> {
        self.world.check_operation_failed()?;
        Ok(self.world.add_data(owner, data))
    }

    /// 显式移除一份数据实例并分发对应原因的销毁事实。
    pub fn remove_data(
        &mut self,
        id: BuffId,
        reason: DestructionReason,
    ) -> Result<bool, OperationError> {
        let changes = ChangeSet::new().destroy(id, reason);
        let result = self.world.submit(changes)?;
        Ok(result.destroyed.iter().any(|d| d.buff_id == id))
    }

    pub fn add_player(
        &mut self,
        player: super::player::Player,
    ) -> Result<PlayerId, OperationError> {
        self.world.check_operation_failed()?;
        Ok(self.world.add_player(player))
    }

    pub fn remove_player(
        &mut self,
        id: PlayerId,
    ) -> Result<Option<super::player::Player>, OperationError> {
        self.world.check_operation_failed()?;
        Ok(self.world.take_player(id))
    }

    /// 按身份读取数据实例。
    pub fn data<T: BuffData>(&self, id: BuffId) -> Option<&T> {
        self.query().data::<T>(id)
    }

    /// 写入（替换）一个中性扩展资源；业务类型与解释逻辑留在业务模块。
    pub fn set_resource<T: Any>(&mut self, value: T) {
        self.world.set_resource(value);
    }

    /// 可变访问一个中性扩展资源。
    pub fn resource_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.world.resource_mut::<T>()
    }

    /// 只读访问一个中性扩展资源。
    pub fn resource<T: Any>(&self) -> Option<&T> {
        self.world.resource::<T>()
    }

    /// 可变访问一个中性扩展资源，不存在时以默认值插入。
    pub fn resource_mut_or_default<T: Any + Default>(&mut self) -> &mut T {
        self.world.resource_mut_or_insert_with(T::default)
    }

    pub fn spawn<O: Operation>(&mut self, operation: O) {
        self.children.push_back(Box::new(operation));
    }

    pub(crate) fn take_children(&mut self) -> VecDeque<Box<dyn ErasedOperation>> {
        std::mem::take(&mut self.children)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HpChange {
    pub target_id: PlayerId,
    pub old_hp: u64,
    pub new_hp: u64,
    pub max_hp: u64,
    pub requested: i64,
    /// 本次提交的无符号数值；伤害和治疗由调用的提交接口区分。
    pub submitted_amount: u64,
}

/// 一次关联提交的候选变化。全部变化在同一个提交边界内写入，
/// 期间不运行玩法响应；随后按“基础状态事实 → 销毁事实”顺序通知。
#[derive(Debug, Default)]
pub struct ChangeSet {
    pub(crate) hp: Option<HpRequest>,
    pub(crate) remove_player: Option<PlayerId>,
    pub(crate) destroy: Vec<(BuffId, DestructionReason)>,
}

/// 无符号的生命变化请求；伤害与治疗分别走有界无符号计算。
#[derive(Debug, Clone, Copy)]
pub(crate) enum HpRequest {
    Damage(PlayerId, u64),
    Heal(PlayerId, u64),
}

impl ChangeSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// 生命变化（正数治疗、负数伤害）。
    pub fn hp(mut self, target_id: PlayerId, modifier: i64) -> Self {
        self.hp = Some(if modifier < 0 {
            HpRequest::Damage(target_id, modifier.unsigned_abs())
        } else {
            HpRequest::Heal(target_id, modifier as u64)
        });
        self
    }

    /// 直接以无符号数值声明伤害，避免大数符号转换。
    pub fn damage(mut self, target_id: PlayerId, amount: u64) -> Self {
        self.hp = Some(HpRequest::Damage(target_id, amount));
        self
    }

    /// 直接以无符号数值声明治疗。
    pub fn heal(mut self, target_id: PlayerId, amount: u64) -> Self {
        self.hp = Some(HpRequest::Heal(target_id, amount));
        self
    }

    /// 移除一名角色；其 owner 依赖的数据实例随同次提交销毁（原因 OwnerDeath）。
    pub fn remove_player(mut self, id: PlayerId) -> Self {
        self.remove_player = Some(id);
        self
    }

    /// 销毁一份数据实例并产生销毁事实。
    pub fn destroy(mut self, id: BuffId, reason: DestructionReason) -> Self {
        self.destroy.push((id, reason));
        self
    }
}

/// 一份被销毁实例的元信息。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DestroyedInfo {
    pub buff_id: BuffId,
    pub owner: Option<PlayerId>,
    pub reason: DestructionReason,
}

#[derive(Debug, Default)]
pub struct SubmissionResult {
    /// 生命变化结果；未声明或目标不存在时为 `None`。
    pub hp: Option<HpChange>,
    /// 声明的角色移除是否实际发生。
    pub player_removed: bool,
    /// 本组提交销毁的全部实例，按 BuffId 升序。
    pub destroyed: Vec<DestroyedInfo>,
}

#[derive(Debug)]
pub struct EmitEvent(pub Event);

impl Operation for EmitEvent {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.publish(self.0);
        completed()
    }
}

#[derive(Debug)]
pub struct AddPlayerOperation(pub super::player::Player);

impl Operation for AddPlayerOperation {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.add_player(self.0)?;
        completed()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RemovePlayerOperation(pub PlayerId);

impl Operation for RemovePlayerOperation {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        if context.remove_player(self.0)?.is_some() {
            completed()
        } else {
            skipped()
        }
    }
}

#[derive(Debug)]
pub struct RemoveDataOperation {
    pub buff_id: BuffId,
    pub reason: DestructionReason,
}

impl Operation for RemoveDataOperation {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        if context.remove_data(self.buff_id, self.reason)? {
            completed()
        } else {
            skipped()
        }
    }
}
