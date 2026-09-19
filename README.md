<p align="center">
  <img src="assets/vulpicast-logo.png" width="180" alt="VulpiCast logo">
</p>

# VulpiCast

VulpiCast plays the sound of your Windows PC on a HomePod or any other AirPlay 2
speaker. Music, podcasts, your browser and other programs are all streamed
together.

## Installation

1. Open the [latest release page](https://github.com/TecTolik/VulpiCast/releases/latest).
2. Download the file **VulpiCast-Setup.exe**.
3. Run the downloaded file and confirm the Windows prompt.
4. Follow the setup wizard and click **Finish** at the end.

Windows may show a security warning the first time, because the installer is not
digitally signed yet. If that happens, choose **More info** and then
**Run anyway**.

## How it works

After installation a small fox icon appears next to the Windows clock in the
bottom right corner. If you cannot see it, click the arrow for hidden icons
first.

1. Right-click the fox icon.
2. Pick one or more speakers from the list. Click a selected speaker again to
   remove it from the active group.
3. Play music or any other sound on your PC.

To stop streaming, choose **Stopped** in the same menu. Under **Settings** you can
change the volume and the keyboard shortcuts. Use **Windows audio output** in
the tray menu to choose which playback device is captured; **System default**
follows the current Windows default device.

## Requirements

- Windows 10 or Windows 11, 64-bit
- a HomePod or another AirPlay 2 speaker
- PC and speaker on the same private Wi-Fi or home network

## If no speaker shows up

1. Check that your PC and the speaker are connected to the same network.
2. Open the fox menu and choose **Rescan for AirPlay 2 devices**.
3. Check in Windows that your network is set to **Private**.
4. Quit VulpiCast and start it again from the Windows Start menu.

If the problem persists, use **Open log folder** to find the diagnostic file and
attach it to a [bug report on GitHub](https://github.com/TecTolik/VulpiCast/issues).

## Uninstalling

Open **Windows Settings → Apps → Installed apps**, look for **VulpiCast** and
choose **Uninstall**. Program files, shortcuts and the firewall rule are removed
automatically.

## Privacy

Audio is streamed entirely within your local network. VulpiCast does not require
an account, contains no ads and sends no usage data to external services.

## Support the project

VulpiCast is built in my spare time and stays free and ad-free. If you like it and
want to support further development, I would be delighted about a small coffee:

<p align="center">
  <a href="https://ko-fi.com/plueten">
    <img src="https://img.shields.io/badge/Ko--fi-Buy%20me%20a%20coffee-FF5E5B?style=for-the-badge&logo=ko-fi&logoColor=white" alt="Support me on Ko-fi">
  </a>
</p>

Every contribution helps — and a star for the project here on GitHub makes me just
as happy. Thank you! 🦊

## Open source

VulpiCast is free software licensed under GPL-2.0. The project builds on
[HomePod Cast](https://github.com/iakacer/windows-airplay-homepod) and
[airplay2-rs](https://github.com/lmcgartland/airplay2-rs). See the
[LICENSE](LICENSE) file for details.

Apple, AirPlay, HomePod and Windows are trademarks of their respective owners.
This project is not affiliated with Apple or Microsoft.
