class Gauge < Formula
  desc "Show remaining Codex and Claude agent quota"
  homepage "https://github.com/manuu-r/homebrew-tap"
  url "https://github.com/manuu-r/homebrew-tap/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "b83a3f0e5bb3ec96287d517802fb27bf0226834138bb9fc4a6dbd36b61f8ecdc"
  license "MIT"
  head "https://github.com/manuu-r/homebrew-tap.git", branch: "main"

  bottle do
    root_url "https://github.com/manuu-r/homebrew-tap/releases/download/v0.1.0"
    rebuild 3
    sha256 cellar: :any_skip_relocation, arm64_sequoia: "78f5535b8ad2fc60bbbd42022e780243fa6ef84c6424b800ecf575785b7ac4fd"
    sha256 cellar: :any_skip_relocation, sequoia:       "e0a6223e50726892fd7ad8aa57f6c068138fc922ec3efae6a336cb464cfbea26"
  end

  depends_on "rust" => :build
  # Reads the macOS keychain via security(1) and drives the macOS menu bar.
  depends_on :macos

  def install
    system "cargo", "install", *std_cargo_args
  end

  def caveats
    <<~EOS
      Gauge prints to stdout by default. For the menu bar item:
        gauge --tray

      To keep it running across logins:
        brew services start gauge
    EOS
  end

  service do
    run [opt_bin/"gauge", "--tray"]
    # Restart on a crash, but respect Quit from the menu, which exits 0.
    keep_alive successful_exit: false
    log_path var/"log/gauge.log"
    error_log_path var/"log/gauge.log"
  end

  test do
    assert_match "gauge #{version}", shell_output("#{bin}/gauge --version")
    assert_match "--tray", shell_output("#{bin}/gauge --help")
  end
end
