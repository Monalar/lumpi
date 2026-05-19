class Lumpi < Formula
  desc "Columnar compression for flat JSONL and CSV logs"
  homepage "https://github.com/Monalar/lumpi"
  version "9.0.5"
  license "Apache-2.0"

  # url and sha256 filled in on first tagged release
  on_macos do
    on_arm do
      url "PLACEHOLDER"
      sha256 "PLACEHOLDER"
    end
    on_intel do
      url "PLACEHOLDER"
      sha256 "PLACEHOLDER"
    end
  end

  on_linux do
    on_intel do
      url "PLACEHOLDER"
      sha256 "PLACEHOLDER"
    end
  end

  def install
    bin.install "lumpi"
  end

  test do
    system "#{bin}/lumpi", "--version"
  end
end
