use std::any::Any;
use std::fmt;

use super::PlayerId;

/// 一份业务数据实例的最小标记协议：只承载数据，可按类型下转型查询。
/// 不包含事件处理方法；作用范围、来源、目标等业务关系由数据字段表达，
/// 基础机制只解释 [`DestructionReason`] 与 owner 的生命周期语义。
pub trait BuffData: Any + fmt::Debug {}

impl<T: Any + fmt::Debug> BuffData for T {}

/// 按具体类型下转型一份数据实例。
/// 参数要求对象类型为 `'static`（std 只在 `dyn Any + 'static` 上提供下转型）；
/// 借用本身可以是任意短生命周期。
pub fn downcast_data<'a, T: Any>(data: &'a (dyn Any + 'static)) -> Option<&'a T> {
    data.downcast_ref::<T>()
}

/// 一条 Buff 记录：外层管理 owner 与稳定顺序，内层是完整业务数据。
/// 本轮不拆分为多个组件。
pub struct BuffRecord {
    /// 生命周期依赖：`Some(A)` 表示 A 正式死亡时本实例立即销毁；
    /// `None` 表示不随任何角色死亡自动销毁。不参与作用范围过滤。
    pub(crate) owner: Option<PlayerId>,
    /// 创建时的单调顺序，参与响应候选的统一排序。
    pub(crate) subject_order: usize,
    pub(crate) data: Box<dyn Any>,
}

impl fmt::Debug for BuffRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BuffRecord")
            .field("owner", &self.owner)
            .field("subject_order", &self.subject_order)
            .finish_non_exhaustive()
    }
}

/// 销毁原因。业务 System 必须按原因判断，不能把普通移除或护盾消耗
/// 都当成死亡爆炸；同一实例只产生一次实际销毁事实。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestructionReason {
    /// owner 正式死亡导致依赖销毁
    OwnerDeath,
    /// 作为某次提交的资源被消耗（如一次性护盾挡下伤害）
    Consumed,
    /// 其他显式移除
    Explicit,
}
