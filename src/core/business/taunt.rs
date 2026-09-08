//! 嘲讽：把攻击目标导向拥有嘲讽数据的存活角色。
use super::super::{engine::BattleEngine, query::Query, ComponentId, PlayerId};

#[derive(Debug)]
pub struct TauntData;

/// 给角色添加嘲讽效果；owner 同时是作用目标和生命周期依赖。
pub fn attach_taunt(engine: &mut BattleEngine, owner: PlayerId) -> ComponentId {
    engine.attach_component(Some(owner), TauntData)
}

/// 返回当前最早创建且仍存活的嘲讽者。
pub fn target(query: &Query<'_>) -> Option<PlayerId> {
    query
        .components::<TauntData>()
        .into_iter()
        .filter_map(|(_, owner, _)| owner)
        .find(|id| query.player(*id).is_some_and(|p| p.is_alive()))
}
