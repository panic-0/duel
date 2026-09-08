//! 战斗引擎提供给 Operation 和 System 的只读查询视图。

use std::any::Any;
use std::collections::{BTreeMap, HashMap};

use super::component::Component;
use super::player::Player;
use super::state::BattleState;
use super::{ComponentId, PlayerId};

/// 交给 System 与规则的只读查询视图：玩家状态、数据实例与扩展资源。
/// 运行期修改仍必须走受控提交，不从这里拿可变引用。
#[derive(Debug)]
pub struct Query<'a> {
    pub(crate) state: &'a BattleState,
    pub(crate) records: &'a BTreeMap<ComponentId, super::component::ComponentRecord>,
    pub(crate) resources: &'a HashMap<std::any::TypeId, Box<dyn Any>>,
}

impl<'a> Query<'a> {
    pub fn state(&self) -> &'a BattleState {
        self.state
    }

    pub fn players(&self) -> &'a BTreeMap<PlayerId, Player> {
        self.state.players()
    }

    pub fn player(&self, id: PlayerId) -> Option<&'a Player> {
        self.state.player(id)
    }

    /// 按身份读取数据实例。
    pub fn component<T: Component>(&self, id: ComponentId) -> Option<&'a T> {
        let record = self.records.get(&id)?;
        record.data.downcast_ref::<T>()
    }

    /// 数据实例的 owner（生命周期依赖），实例不存在时返回 `None`。
    pub fn owner_of(&self, id: ComponentId) -> Option<Option<PlayerId>> {
        self.records.get(&id).map(|record| record.owner)
    }

    /// 实例创建时的单调顺序，用于候选排序。
    pub fn subject_order(&self, id: ComponentId) -> Option<usize> {
        self.records.get(&id).map(|record| record.subject_order)
    }

    /// 按类型收集全部数据实例，按 ComponentId 升序返回。
    pub fn components<T: Component>(&self) -> Vec<(ComponentId, Option<PlayerId>, &'a T)> {
        self.records
            .iter()
            .filter_map(|(id, record)| {
                let data = record.data.downcast_ref::<T>()?;
                Some((*id, record.owner, data))
            })
            .collect()
    }

    /// 只读访问扩展资源（如业务规则注册表）。
    pub fn resource<T: Any>(&self) -> Option<&'a T> {
        self.resources
            .get(&std::any::TypeId::of::<T>())
            .and_then(|value| value.downcast_ref::<T>())
    }
}
