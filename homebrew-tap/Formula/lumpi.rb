class Lumpi < Formula
  desc "Columnar compression for flat JSONL and CSV logs"
  homepage "https://github.com/Monalar/lumpi"
  version "9.0.4"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/Monalar/lumpi/releases/download/v9.0.4/lumpi-aarch64-apple-darwin.tar.gz"
      sha256 "TODO"
    end
    on_intel do
      url "https://github.com/Monalar/lumpi/releases/download/v9.0.4/lumpi-x86_64-apple-darwin.tar.gz"
      sha256 "TODO"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Monalar/lumpi/releases/download/v9.0.4/lumpi-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "TODO"
    end
  end

  def install
    bin.install "lumpi"
  end

  test do
    system "#{bin}/lumpi", "--version"
  end
end
