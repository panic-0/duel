//! 世界状态、初始化装配与查询入口。
//! 执行调度、通知分发和关联提交分别由私有子模块实现。

mod dispatch;
mod execution;
mod submission;

use crate::core::{
    buff_data::{BuffData, BuffRecord},
    log::{LogEntry, Logger},
    operation::OperationError,
    player::Player,
    query::Query,
    state::GameState,
    system::{NoticeKind, Priority, System},
    BuffId, PlayerId,
};
use std::{
    any::{Any, TypeId},
    collections::{BTreeMap, HashMap},
};

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
    /// 设置初始化阶段的生命值；运行期生命变化必须通过受控提交。
    pub fn set_initial_hp(&mut self, id: PlayerId, hp: u64) -> bool {
        self.state
            .get_player_mut(id)
            .map(|player| {
                player.set_hp(hp);
            })
            .is_some()
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

    /// 只读访问一个中性扩展资源。失败后仍允许读取与诊断。
    pub fn resource<T: Any>(&self) -> Option<&T> {
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

    /// 从共同单调顺序源取下一个独立响应主体顺序。
    /// 供业务注册表登记独立候选身份，避免与实例创建顺序使用不同计数空间。
    pub(crate) fn next_subject_order(&mut self) -> usize {
        self.next_subject()
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
