class Fflume < Formula
  desc "Native desktop toolkit for image, video, audio, and PDF conversion"
  # Release placeholders: replace the repository and URL tag, then add the actual archive SHA-256.
  homepage "https://github.com/REPLACE_OWNER/REPLACE_REPO"
  url "https://github.com/REPLACE_OWNER/REPLACE_REPO/archive/refs/tags/v0.2.0.tar.gz"
  # sha256 "REPLACE_WITH_OFFICIAL_ARCHIVE_SHA256"
  license "GPL-3.0-only"

  depends_on "rust" => :build
  depends_on :macos
  depends_on "ffmpeg"

  deny_network_access!

  def fetch
    system "cargo", "fetch", *std_cargo_fetch_args
  end

  def install
    ENV["FFLUME_BREW_FFMPEG_DIR"] = Formula["ffmpeg"].opt_bin.to_s
    system "cargo", "install", *std_cargo_args
    (pkgshare/"licenses").install "LICENSE", "assets/fonts/Geist-OFL.txt", "assets/fonts/Geist-LICENSE.txt"
  end

  test do
    assert_equal "fflume #{version}\n", shell_output("#{bin}/fflume --version")
  end
end
