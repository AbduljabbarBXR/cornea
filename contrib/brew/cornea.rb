# Homebrew formula for Cornea.
#
# Copy this into a `homebrew-cornea` tap repo (e.g. AbduljabbarBXR/homebrew-cornea)
# as `Formula/cornea.rb`, replacing the two `url`/`sha256` placeholders with the
# per-version asset URLs/checksums from the GitHub Release, OR keep it in
# contrib/ and install locally with:
#
#   brew install --formula contrib/brew/cornea.rb
#
# For automated tap publishing, wire the release workflow to regenerate this
# file on each release (see PUBLISHING.md).

class Cornea < Formula
  desc "Deterministic visual inspection engine + MCP server for AI agents"
  homepage "https://github.com/AbduljabbarBXR/cornea"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/AbduljabbarBXR/cornea/releases/download/v0.1.0/aarch64-apple-darwin.tar.gz"
      sha256 "<REPLACE_ME_aarch64-apple-darwin>"
    elsif Hardware::CPU.intel?
      url "https://github.com/AbduljabbarBXR/cornea/releases/download/v0.1.0/x86_64-apple-darwin.tar.gz"
      sha256 "<REPLACE_ME_x86_64-apple-darwin>"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/AbduljabbarBXR/cornea/releases/download/v0.1.0/aarch64-unknown-linux-gnu.tar.gz"
      sha256 "<REPLACE_ME_aarch64-unknown-linux-gnu>"
    elsif Hardware::CPU.intel?
      url "https://github.com/AbduljabbarBXR/cornea/releases/download/v0.1.0/x86_64-unknown-linux-gnu.tar.gz"
      sha256 "<REPLACE_ME_x86_64-unknown-linux-gnu>"
    end
  end

  def install
    bin.install "cornea"
  end

  test do
    # --version proves the binary runs and reports its version.
    assert_match "cornea #{version}", shell_output("#{bin}/cornea --version")
  end
end
