#!/bin/bash

set -eo pipefail

REPO_OWNER="skyline69"
REPO_NAME="spotix"

cat <<EOF
cask "spotix" do
  version :latest
  sha256 :no_check

  url "https://github.com/${REPO_OWNER}/${REPO_NAME}/releases/latest/download/Spotix.dmg"
  name "Spotix"
  desc "Fast and native Spotify client"
  homepage "https://github.com/${REPO_OWNER}/${REPO_NAME}/"

  depends_on macos: ">= :big_sur"

  app "Spotix.app"

  zap trash: [
    "~/Library/Application Support/Spotix",
    "~/Library/Caches/com.skyline69.spotix",
    "~/Library/Caches/Spotix",
    "~/Library/HTTPStorages/com.skyline69.spotix",
    "~/Library/Preferences/com.skyline69.spotix.plist",
    "~/Library/Saved Application State/com.skyline69.spotix.savedState",
  ]
end
EOF
