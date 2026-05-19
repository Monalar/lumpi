class Lumpi < Formula
  desc "Columnar compression for flat JSONL and CSV logs"
  homepage "https://github.com/Monalar/lumpi"
  version "9.0.5"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/Monalar/lumpi/releases/download/v9.0.4/lumpi-aarch64-apple-darwin.tar.gz"
      sha256 "1ff1503886b9c42ca3709ab421b7e6c2aa6b2338f112cbad82cee2212adf45d7"
    end
    on_intel do
      url "https://github.com/Monalar/lumpi/releases/download/v9.0.4/lumpi-x86_64-apple-darwin.tar.gz"
      sha256 "8838b229581ef1bf332cb7e99bde4962b48e33091222dbd28c910e8cf5eb0246"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Monalar/lumpi/releases/download/v9.0.4/lumpi-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "d2320397c25ff923d3881448d3bdb6a06d1efecfd348729380b97d9b37b4143c"
    end
  end

  def install
    bin.install "lumpi"
  end

  test do
    system "#{bin}/lumpi", "--version"
  end
end
