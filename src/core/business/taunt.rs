//! 嘲讽：把攻击目标导向拥有嘲讽数据的存活角色。
use super::super::{query::Query, world::World, BuffId, PlayerId};

#[derive(Debug)]
pub struct TauntData;

/// 给角色添加嘲讽效果；owner 同时是作用目标和生命周期依赖。
pub fn add_taunt(world: &mut World, owner: PlayerId) -> BuffId {
    world.add_data(Some(owner), TauntData)
}

/// 返回当前最早创建且仍存活的嘲讽者。
pub fn target(query: &Query<'_>) -> Option<PlayerId> {
    query
        .instances::<TauntData>()
        .into_iter()
        .filter_map(|(_, owner, _)| owner)
        .find(|id| query.player(*id).is_some_and(|p| p.is_alive()))
}
