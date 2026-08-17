export default function AboutTab() {
  return (
    <>
      <div class="section-label">About</div>
      <div class="about-text">
{`Hacker Launcher v1.0.0
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

A game launcher for running Windows games (Proton, Wine, Flatpak,
Steam, Native) and Android apps (via Waydroid) on Linux.

Built with:
  • Rust + Tauri (backend)
  • SolidJS + TypeScript (frontend)

GitHub: https://github.com/HackerOS-Linux-System/Hacker_Launcher

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Features:
  • Add and manage games with multiple runners
  • Install and manage Proton GE / Official versions
  • Per-game DXVK, Esync, Fsync, DXVK-Async options
  • Per-game custom environment variables
  • Gamescope integration (--gamescope launch option)
  • Wine prefix management, plus optional shared prefix across games
  • FPS limiting via Gamescope
  • Live "running" status, stop button, and playtime tracking
  • Rotating per-run game logs (last 10 runs kept)
  • Proton install progress with checksum verification, changelog preview & cancel support
  • Light/Dark theme, List/Grid library views, tags & favorites
  • Drag & drop a .exe to add it as a game; Enter/Delete keyboard shortcuts
  • Steam library auto-import, ProtonDB compatibility lookup
  • Winetricks and dependency-installer integration
  • Backup/restore games & settings as JSON
  • System notifications when background installs finish

Data stored in: ~/.hackeros/Hacker-Launcher/
`}
      </div>
    </>
  );
}
