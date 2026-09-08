//! 操作可使用的查询、提交与子操作调度能力。

use super::{
    ChangeSet, ErasedOperation, HpChange, Operation, OperationError, OperationOutcome,
    SubmissionResult,
};
use crate::core::{
    buff_data::{BuffData, DestructionReason},
    event::Event,
    query::Query,
    state::GameState,
    world::World,
    BuffId, PlayerId,
};
use std::{any::Any, collections::VecDeque};

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

    /// 发布事件并等待其全部响应完成。错误记入世界后由本调用返回；
    /// 后续受控入口（提交、子操作）也会拒绝执行，根调用最终返回该错误。
    pub fn publish(&mut self, event: Event) -> Result<(), OperationError> {
        self.try_publish(event)
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

    pub fn log(&self, entry: crate::core::log::LogEntry) {
        self.world.log(entry);
    }

    pub fn is_end(&self) -> bool {
        self.world.is_end()
    }

    /// 结束对局。已终局时重复调用为无副作用成功。
    pub fn end_game(
        &mut self,
        result: crate::core::world::GameResult,
    ) -> Result<(), OperationError> {
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
    /// 运行期校验生命周期依赖：`Some(owner)` 指向的角色必须仍然存在，
    /// 否则拒绝创建——D4 允许已产生的操作继续执行，但生命周期资格不豁免。
    pub fn add_data(
        &mut self,
        owner: Option<PlayerId>,
        data: impl BuffData + 'static,
    ) -> Result<BuffId, OperationError> {
        self.world.check_operation_failed()?;
        if let Some(owner_id) = owner {
            if self.world.get_player(owner_id).is_none() {
                return Err(OperationError::Invalid(format!(
                    "生命周期依赖的角色 {owner_id} 已不存在，拒绝创建依赖数据"
                )));
            }
        }
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
        player: crate::core::player::Player,
    ) -> Result<PlayerId, OperationError> {
        self.world.check_operation_failed()?;
        Ok(self.world.add_player(player))
    }

    /// 移除一名角色。与 ChangeSet 的角色移除走同一条生命周期路径：
    /// owner 依赖的数据实例随同次提交销毁，并产生相应销毁事实。
    pub fn remove_player(&mut self, id: PlayerId) -> Result<bool, OperationError> {
        self.world.check_operation_failed()?;
        let result = self.world.submit(ChangeSet::new().remove_player(id))?;
        Ok(result.player_removed)
    }

    /// 受控更新一份数据实例：保留原身份与候选顺序，就地替换数据。
    /// 更新在关联提交的写入阶段完成，不产生销毁事实。
    pub fn update_data(
        &mut self,
        id: BuffId,
        data: impl BuffData + 'static,
    ) -> Result<bool, OperationError> {
        self.world.check_operation_failed()?;
        let changes = ChangeSet::new().update_data(id, Box::new(data));
        let result = self.world.submit(changes)?;
        Ok(result.updated.contains(&id))
    }

    /// 按身份读取数据实例。
    pub fn data<T: BuffData>(&self, id: BuffId) -> Option<&T> {
        self.query().data::<T>(id)
    }

    /// 写入（替换）一个中性扩展资源；业务类型与解释逻辑留在业务模块。
    /// 世界已记录失败时拒绝写入。
    pub fn set_resource<T: Any>(&mut self, value: T) -> Result<(), OperationError> {
        self.world.check_operation_failed()?;
        self.world.set_resource(value);
        Ok(())
    }

    /// 可变访问一个中性扩展资源。世界已记录失败时拒绝写入。
    pub fn resource_mut<T: Any>(&mut self) -> Result<Option<&mut T>, OperationError> {
        self.world.check_operation_failed()?;
        Ok(self.world.resource_mut::<T>())
    }

    /// 只读访问一个中性扩展资源。失败后仍允许读取与诊断。
    pub fn resource<T: Any>(&self) -> Option<&T> {
        self.world.resource::<T>()
    }

    /// 可变访问一个中性扩展资源，不存在时以默认值插入。
    /// 世界已记录失败时拒绝写入。
    pub fn resource_mut_or_default<T: Any + Default>(&mut self) -> Result<&mut T, OperationError> {
        self.world.check_operation_failed()?;
        Ok(self.world.resource_mut_or_insert_with(T::default))
    }

    /// 失败后的受限清理路径：仅供“进行中标记必须释放”一类必要收尾使用，
    /// 不承载任何新的玩法状态变化。
    pub(crate) fn resource_mut_ignoring_failure<T: Any>(&mut self) -> Option<&mut T> {
        self.world.resource_mut::<T>()
    }

    pub fn spawn<O: Operation>(&mut self, operation: O) {
        self.children.push_back(Box::new(operation));
    }

    pub(crate) fn take_children(&mut self) -> VecDeque<Box<dyn ErasedOperation>> {
        std::mem::take(&mut self.children)
    }
}
