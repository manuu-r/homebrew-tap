class Gauge < Formula
  desc "Show remaining Codex and Claude agent quota"
  homepage "https://github.com/manuu-r/homebrew-tap"
  url "https://github.com/manuu-r/homebrew-tap/archive/refs/tags/v0.1.5.tar.gz"
  sha256 "060935c643b417302d7e5078db1fa2864c875f0aaa4a1e9812ab99cf05726aa0"
  license "MIT"
  head "https://github.com/manuu-r/homebrew-tap.git", branch: "main"

  depends_on "rust" => :build
  # Reads the macOS keychain via security(1) and drives the macOS menu bar.
  depends_on :macos

  def install
    system "cargo", "install", *std_cargo_args

    app = prefix/"Gauge.app"
    binary = bin/"gauge"
    (app/"Contents/MacOS").install binary
    (app/"Contents/Resources").install "packaging/Gauge.icns"
    (app/"Contents").install "packaging/Info.plist"
    inreplace app/"Contents/Info.plist", "<string>0.1.5</string>",
              "<string>#{version}</string>"
    system "codesign", "--force", "--deep", "--sign", "-", app

    # Keep the existing command-line snapshot utility available. Opening the
    # app bundle with no arguments starts the menu-bar app instead.
    bin.install_symlink app/"Contents/MacOS/gauge"
  end

  def caveats
    <<~EOS
      Open the standalone menu-bar app:
        open "#{opt_prefix}/Gauge.app"

      To keep it running across logins:
        brew services start gauge
    EOS
  end

  service do
    run [opt_prefix/"Gauge.app/Contents/MacOS/gauge", "--tray"]
    # Restart on a crash, but respect Quit from the menu, which exits 0.
    keep_alive successful_exit: false
    log_path var/"log/gauge.log"
    error_log_path var/"log/gauge.log"
  end

  test do
    assert_match "gauge #{version}", shell_output("#{bin}/gauge --version")
    assert_match "--tray", shell_output("#{bin}/gauge --help")
    assert_path_exists prefix/"Gauge.app/Contents/Info.plist"
    assert_path_exists prefix/"Gauge.app/Contents/Resources/Gauge.icns"
  end
end
