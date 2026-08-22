class Gauge < Formula
  desc "Show remaining Codex and Claude agent quota"
  homepage "https://github.com/manuu-r/homebrew-tap"
  url "https://github.com/manuu-r/homebrew-tap/archive/refs/tags/v0.1.2.tar.gz"
  sha256 "9a116635ec5f9f2722ae39a59923fc01eeea39a4f1db43177cb294e585c335ee"
  license "MIT"
  head "https://github.com/manuu-r/homebrew-tap.git", branch: "main"

  bottle do
    root_url "https://github.com/manuu-r/homebrew-tap/releases/download/v0.1.2"
    rebuild 1
    sha256 cellar: :any_skip_relocation, arm64_sequoia: "98384e173d7c919d6c69c2ad90f05ee84bde14ab361d719fa5ed91f062c4506e"
    sha256 cellar: :any_skip_relocation, sequoia:       "874b23a7893683c946291af1eb316ca9ee0f8e8772003cdc0109354a0ba38609"
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
