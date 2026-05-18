class Lumpi < Formula
  desc "Columnar compression for flat JSONL and CSV logs"
  homepage "https://github.com/Monalar/lumpi"
  version "9.0.4"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/Monalar/lumpi/releases/download/v9.0.4/lumpi-aarch64-apple-darwin.tar.gz"
      sha256 "e706ba0c4a9c53cbd9851634b9a76fd40781f2627dbf294ceaf149d6d1af03c4"
    end
    on_intel do
      url "https://github.com/Monalar/lumpi/releases/download/v9.0.4/lumpi-x86_64-apple-darwin.tar.gz"
      sha256 "47d0d27e0acf19cc11337c6d085ac5bcc6d0c5dac8c265629450ab2d115da273"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Monalar/lumpi/releases/download/v9.0.4/lumpi-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "9888238f5a00edb2faf387d81d78bd029e68017feefc92d8b04e07101a1d21db"
    end
  end

  def install
    bin.install "lumpi"
  end

  test do
    system "#{bin}/lumpi", "--version"
  end
end
