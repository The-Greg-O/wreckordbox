{
  lib,
  makeDesktopItem,
}:

let
  # Cargo package name (`src-tauri/Cargo.toml`) keeps upstream's name for now;
  # the built desktop binary is `wreckordbox-desktop`.
  pname = "rbxport";
  binName = "wreckordbox-desktop";

  icon = file: "${../..}/src-tauri/icons/${file}";

  desktopItem = makeDesktopItem {
    name = "wreckordbox";
    desktopName = "wreckordbox";
    comment = "DJ library and USB export manager";
    exec = binName;
    icon = "wreckordbox";
    startupWMClass = binName;
    terminal = false;
    categories = [
      "AudioVideo"
      "Audio"
      "Music"
    ];
  };
in
{
  inherit pname icon desktopItem;

  installDesktop = ''
    install -Dm644 ${desktopItem}/share/applications/wreckordbox.desktop \
      "$out/share/applications/wreckordbox.desktop"
    install -Dm644 ${icon "32x32.png"} "$out/share/icons/hicolor/32x32/apps/wreckordbox.png"
    install -Dm644 ${icon "128x128.png"} "$out/share/icons/hicolor/128x128/apps/wreckordbox.png"
    install -Dm644 ${icon "128x128@2x.png"} "$out/share/icons/hicolor/256x256@2/apps/wreckordbox.png"
  '';

  meta = {
    description = "Linux-first DJ library and USB export app, forked from rbxport";
    longDescription = ''
      Manage a rekordbox-compatible DJ library, run a background Library
      service for the desktop window, CLI, and MCP server, and load playable
      USB Sticks.
    '';
    homepage = "https://github.com/The-Greg-O/wreckordbox";
    license = lib.licenses.gpl2Plus;
    mainProgram = binName;
    platforms = [ "x86_64-linux" "aarch64-linux" ];
  };
}
