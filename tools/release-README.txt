Cozy Rally {{VERSION}} (build {{HASH}}, {{DATE}})
=====================================

CozyRally-windows-x64.exe      The game for Windows. A single file with no install: just run it.
CozyRally-android-arm64.apk    The game for Android phones (arm64, Android 8+). Sideload it.
rally_server-windows-x64.exe   Online race server for Windows (console).
rally_server-linux-x64         Online race server for Linux x86-64 (glibc 2.35+).

PC and phone players can race each other on the same server.


Install on Android
------------------
Copy the APK to the phone and open it; Android asks once to allow installs from that app.
Or from a PC:  adb install -r CozyRally-android-arm64.apk
This build is signed with a debug key. A later build signed with a different key won't install
over it: uninstall first, which deletes the phone's saves.


Host an online race
-------------------
1. On the host machine, run the server:
     rally_server-windows-x64.exe                (Windows; allow it through the firewall)
     ./rally_server-linux-x64                    (Linux)
   Options: --port 27420 --name "My server"
2. The server prints the address friends should type in. In the game: Online, then Server
   (the address), Your name, Connect. The first player in is the leader: they pick the race,
   everyone picks a car, and the leader starts.

Addresses:
  - Same Wi-Fi / LAN: the host's LAN IP, which the server prints at startup.
  - Over the internet: a Tailscale / ZeroTier IP, or the host's public IP with port 27420
    forwarded on the router (TCP and UDP).
UDP carries the car positions. If UDP is blocked, the game falls back to TCP automatically
(the lobby shows UDP or TCP for each player).


Checksums: SHA256SUMS.txt
