class Gauge < Formula
  desc "Show remaining Codex and Claude agent quota"
  homepage "https://github.com/manuu-r/homebrew-tap"
  url "https://github.com/manuu-r/homebrew-tap/archive/refs/tags/v0.1.4.tar.gz"
  sha256 "c263a77bb63fe01ef595d3a9870b9756482581fcd6250c856413e8cb87ca5c80"
  license "MIT"
  head "https://github.com/manuu-r/homebrew-tap.git", branch: "main"

  bottle do
    root_url "https://github.com/manuu-r/homebrew-tap/releases/download/v0.1.4"
    rebuild 1
    sha256 cellar: :any_skip_relocation, arm64_sequoia: "98c05e6c965210da6a3358d511d03f47bd88aec4b3b6d68ecb0c25d0673a9f59"
    sha256 cellar: :any_skip_relocation, sequoia:       "fb05bab0bfbb1fc64c92fcdad1947484cb8ef8fd531d047085893fb87e3c20c2"
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
