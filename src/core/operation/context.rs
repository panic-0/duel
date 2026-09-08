//! 操作可使用的查询、提交与子操作调度能力。

use super::{
    ChangeSet, ErasedOperation, HpChange, Operation, OperationError, OperationOutcome,
    SubmissionResult,
};
use crate::core::{
    component::{Component, DestructionReason},
    engine::BattleEngine,
    event::Event,
    query::Query,
    state::BattleState,
    ComponentId, PlayerId,
};
use std::{any::Any, collections::VecDeque};

/// 交给操作的受限能力集合。引擎本身不暴露，
/// 因此操作无法任意修改状态或重入驱动器。
pub struct ActionContext<'a> {
    engine: &'a mut BattleEngine,
    children: VecDeque<Box<dyn ErasedOperation>>,
}

impl<'a> ActionContext<'a> {
    pub(crate) fn new(engine: &'a mut BattleEngine) -> Self {
        Self {
            engine,
            children: VecDeque::new(),
        }
    }

    pub fn state(&self) -> &BattleState {
        self.engine.state_view()
    }

    /// 只读查询视图：玩家状态、数据实例与扩展资源。
    pub fn query(&self) -> Query<'_> {
        self.engine.query()
    }

    /// 发布事件并等待其全部响应完成。错误记入世界后由本调用返回；
    /// 后续受控入口（提交、子操作）也会拒绝执行，根调用最终返回该错误。
    pub fn publish(&mut self, event: Event) -> Result<(), OperationError> {
        self.try_publish(event)
    }

    /// 发布事件并等待其全部响应完成；事件是不可变事实，没有读回值。
    pub fn try_publish(&mut self, event: Event) -> Result<(), OperationError> {
        self.engine.settle_operation_event(event)
    }

    /// 同步执行子操作；返回时，子操作及其事件响应均已完成。
    pub fn execute<O: Operation>(
        &mut self,
        operation: O,
    ) -> Result<OperationOutcome, OperationError> {
        self.engine.execute_child(Box::new(operation))
    }

    /// 同步执行类型擦除后的子操作。
    pub fn execute_boxed(
        &mut self,
        operation: Box<dyn Operation>,
    ) -> Result<OperationOutcome, OperationError> {
        self.engine.execute_child(Box::new(operation))
    }

    pub fn log(&self, entry: crate::core::log::LogEntry) {
        self.engine.log(entry);
    }

    pub fn is_end(&self) -> bool {
        self.engine.is_end()
    }

    /// 结束对局。已终局时重复调用为无副作用成功。
    pub fn end_game(
        &mut self,
        result: crate::core::engine::BattleResult,
    ) -> Result<(), OperationError> {
        self.engine.end_game(result)
    }

    /// 中性关联提交：本组变化全部写入后才按约定顺序开放通知。
    pub fn submit(&mut self, changes: ChangeSet) -> Result<SubmissionResult, OperationError> {
        self.engine.submit(changes)
    }

    /// 提交一次生命修改（单条中性变化）并完成其事件响应。
    /// `Ok(None)` 表示目标不存在（可解释的跳过）；`Err` 表示响应链失败且已记录。
    pub fn modify_hp(
        &mut self,
        target_id: PlayerId,
        modifier: i64,
    ) -> Result<Option<HpChange>, OperationError> {
        let changes = ChangeSet::new().hp(target_id, modifier);
        Ok(self.engine.submit(changes)?.hp)
    }
    /// 注册一份数据实例，返回稳定身份；销毁事实由后续提交产生。
    /// 运行期校验生命周期依赖：`Some(owner)` 指向的角色必须仍然存在，
    /// 否则拒绝创建——已产生的操作继续执行，但生命周期资格不豁免。
    pub fn attach_component(
        &mut self,
        owner: Option<PlayerId>,
        data: impl Component + 'static,
    ) -> Result<ComponentId, OperationError> {
        self.engine.check_operation_failed()?;
        if let Some(owner_id) = owner {
            if self.engine.player(owner_id).is_none() {
                return Err(OperationError::Invalid(format!(
                    "生命周期依赖的角色 {owner_id} 已不存在，拒绝创建依赖数据"
                )));
            }
        }
        Ok(self.engine.attach_component(owner, data))
    }

    /// 显式移除一份数据实例并分发对应原因的销毁事实。
    pub fn remove_component(
        &mut self,
        id: ComponentId,
        reason: DestructionReason,
    ) -> Result<bool, OperationError> {
        let changes = ChangeSet::new().destroy(id, reason);
        let result = self.engine.submit(changes)?;
        Ok(result.destroyed.iter().any(|d| d.buff_id == id))
    }

    pub fn add_player(
        &mut self,
        player: crate::core::player::Player,
    ) -> Result<PlayerId, OperationError> {
        self.engine.check_operation_failed()?;
        Ok(self.engine.add_player(player))
    }

    /// 移除一名角色。与 ChangeSet 的角色移除走同一条生命周期路径：
    /// owner 依赖的数据实例随同次提交销毁，并产生相应销毁事实。
    pub fn remove_player(&mut self, id: PlayerId) -> Result<bool, OperationError> {
        self.engine.check_operation_failed()?;
        let result = self.engine.submit(ChangeSet::new().remove_player(id))?;
        Ok(result.player_removed)
    }

    /// 受控更新一份数据实例：保留原身份与候选顺序，就地替换数据。
    /// 更新在关联提交的写入阶段完成，不产生销毁事实。
    pub fn update_component(
        &mut self,
        id: ComponentId,
        data: impl Component + 'static,
    ) -> Result<bool, OperationError> {
        self.engine.check_operation_failed()?;
        let changes = ChangeSet::new().update_component(id, Box::new(data));
        let result = self.engine.submit(changes)?;
        Ok(result.updated.contains(&id))
    }

    /// 按身份读取数据实例。
    pub fn component<T: Component>(&self, id: ComponentId) -> Option<&T> {
        self.query().component::<T>(id)
    }

    /// 写入（替换）一个中性扩展资源；业务类型与解释逻辑留在业务模块。
    /// 引擎已记录失败时拒绝写入。
    pub fn set_resource<T: Any>(&mut self, value: T) -> Result<(), OperationError> {
        self.engine.check_operation_failed()?;
        self.engine.set_resource(value);
        Ok(())
    }

    /// 可变访问一个中性扩展资源。引擎已记录失败时拒绝写入。
    pub fn resource_mut<T: Any>(&mut self) -> Result<Option<&mut T>, OperationError> {
        self.engine.check_operation_failed()?;
        Ok(self.engine.resource_mut::<T>())
    }

    /// 只读访问一个中性扩展资源。失败后仍允许读取与诊断。
    pub fn resource<T: Any>(&self) -> Option<&T> {
        self.engine.resource::<T>()
    }

    /// 可变访问一个中性扩展资源，不存在时以默认值插入。
    /// 引擎已记录失败时拒绝写入。
    pub fn resource_mut_or_default<T: Any + Default>(&mut self) -> Result<&mut T, OperationError> {
        self.engine.check_operation_failed()?;
        Ok(self.engine.resource_mut_or_insert_with(T::default))
    }

    /// 失败后的受限清理路径：仅供“进行中标记必须释放”一类必要收尾使用，
    /// 不承载任何新的玩法状态变化。
    pub(crate) fn resource_mut_ignoring_failure<T: Any>(&mut self) -> Option<&mut T> {
        self.engine.resource_mut::<T>()
    }

    pub fn spawn<O: Operation>(&mut self, operation: O) {
        self.children.push_back(Box::new(operation));
    }

    pub(crate) fn take_children(&mut self) -> VecDeque<Box<dyn ErasedOperation>> {
        std::mem::take(&mut self.children)
    }
}
