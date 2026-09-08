use duel::core::{engine::MAX_ROUNDS, log::LogEntry, state::BattleState, PlayerId};

pub(super) fn print_log(world: &BattleState, entry: &LogEntry) {
    use colored::Colorize;
    let name = |id: PlayerId| {
        world
            .player(id)
            .map(|p| p.name().to_string())
            .unwrap_or_default()
    };
    match entry {
        LogEntry::Attack {
            source_id,
            target_id,
        } => println!(
            "{} 对 {} 进行了普通攻击",
            name(*source_id).blue(),
            name(*target_id).blue()
        ),
        LogEntry::DamageReduced {
            target_id,
            original,
            reduced,
        } => println!(
            "{} 的减伤效果触发！伤害从 {} 降低到 {}",
            name(*target_id).blue(),
            original.to_string().bright_red(),
            reduced.to_string().yellow()
        ),
        LogEntry::Damage {
            target_id,
            amount,
            hp,
            max_hp,
        } => println!(
            "{} 受到了 {} 点伤害 ({}/{} HP)",
            name(*target_id).blue(),
            amount.to_string().bright_red(),
            hp,
            max_hp
        ),
        LogEntry::Heal {
            target_id,
            amount,
            hp,
            max_hp,
        } => println!(
            "{} 回复了 {} 点生命 ({}/{} HP)",
            name(*target_id).blue(),
            amount.to_string().bright_green(),
            hp,
            max_hp
        ),
        LogEntry::Revival { player_id } => println!("{} 复活了！", name(*player_id).blue()),
        LogEntry::Death { player_id } => println!("{} 已经死亡", name(*player_id).bright_red()),
        LogEntry::Draw => println!("已达到最大回合数（{}），平局", MAX_ROUNDS),
    }
}
