//! 事件响应 System、事件事实包装和反应目标。

use std::any::Any;

use super::component::DestructionReason;
use super::event::Event;
pub use super::event::EventKind;
use super::operation::{Operation, OperationError};
use super::query::Query;
use super::{ComponentId, PlayerId};

/// 通知分发优先级：变体声明顺序即触发顺序（小者先触发）。
/// 同级内部按统一候选键排序，不按“全局／局部”划分特权阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// 修改事件参数（如减伤改写伤害值），在其他监听者之前触发
    Modify,
    /// 常规监听
    Default,
    /// 结算（消费已被修改过的事件参数）
    Resolve,
    /// 兜底阶段，晚于结算触发（引擎的流程推进在所有响应之后进行）
    Final,
}

/// 响应主体：数据实例候选，或独立 System 的不绑定候选。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReactionTarget {
    Standalone,
    Instance(ComponentId),
}

/// 销毁事实的只读视图。被销毁数据由本次提交暂存，分发期间只读借用；
/// System 应让 Operation 携带独立参数返回，不保留对它的引用。
#[derive(Debug)]
pub struct Destruction<'a> {
    pub buff_id: ComponentId,
    pub owner: Option<PlayerId>,
    pub reason: DestructionReason,
    pub data: &'a (dyn Any + 'static),
}

/// System 读到的只读事实：普通事件或销毁事实，共用同一分发与排序机制。
#[derive(Debug)]
pub enum EventEnvelope<'a> {
    Event(&'a Event),
    Destroyed(Destruction<'a>),
}

impl<'a> EventEnvelope<'a> {
    /// 普通事件的便捷访问；销毁事实返回 `None`。
    pub fn event(&self) -> Option<&'a Event> {
        match self {
            EventEnvelope::Event(event) => Some(event),
            EventEnvelope::Destroyed(_) => None,
        }
    }
}

/// 业务响应逻辑：根据事件或事实查询数据、判断条件，
/// 修改允许修改的参数（独立窗口）或提出 Operation。
/// 不包含每个实例的次数、冷却等可变游戏状态；可保存不可变配置。
///
/// 分为候选收集与逐候选响应两步：收集时固定候选身份与统一顺序，
/// 轮到候选时才读取最新状态判断；一个 System 持有多个实例时，
/// 必须逐候选响应，与其他 System 的候选交错排序，不能批量处理完再轮到别人。
pub trait System: std::fmt::Debug {
    /// 声明订阅的通知与优先级。
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        Vec::new()
    }

    /// 收集本 System 对该事实的候选主体。
    /// 只确定候选身份与业务范围，不冻结生命、次数等可变条件。
    fn candidates(&self, _fact: &EventEnvelope<'_>, _query: &Query<'_>) -> Vec<ReactionTarget> {
        Vec::new()
    }

    /// 响应一个候选，提出有序 Operation。
    /// 返回后即不再借用被销毁数据；Operation 应携带独立参数。
    fn respond(
        &self,
        _fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        _query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(Vec::new())
    }
}
