class Gauge < Formula
  desc "Show remaining Codex and Claude agent quota"
  homepage "https://github.com/manuu-r/homebrew-tap"
  url "https://github.com/manuu-r/homebrew-tap/archive/refs/tags/v0.1.3.tar.gz"
  sha256 "4d754514b4dc00cf2894e81aa1121afadfc888cdd63c4d970734d4d0cbab13aa"
  license "MIT"
  head "https://github.com/manuu-r/homebrew-tap.git", branch: "main"

  bottle do
    root_url "https://github.com/manuu-r/homebrew-tap/releases/download/v0.1.3"
    rebuild 1
    sha256 cellar: :any_skip_relocation, arm64_sequoia: "3cb4ae4931369da4db3359f960c73e2dab03a341a71188bcc87813c44d90aebe"
    sha256 cellar: :any_skip_relocation, sequoia:       "739e427da2871b7d25a87f40c44e030a1cfb71993b7756ab536b2d23d51cbcad"
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
