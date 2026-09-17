<p align="center">
  <img src="assets/vulpicast-logo.png" width="180" alt="VulpiCast-Logo">
</p>

# VulpiCast

Mit VulpiCast hörst du den Ton deines Windows-PCs über einen HomePod oder einen
anderen AirPlay-2-Lautsprecher. Musik, Podcasts, Browser und andere Programme
werden gemeinsam übertragen.

## Installation

1. Öffne die Seite [Neueste Version herunterladen](https://github.com/TecTolik/VulpiCast/releases/latest).
2. Lade die Datei **VulpiCast-Setup.exe** herunter.
3. Öffne die heruntergeladene Datei und bestätige die Windows-Rückfrage.
4. Folge dem Installationsassistenten und klicke am Ende auf **Fertigstellen**.

Windows kann beim ersten Start eine Sicherheitswarnung anzeigen, weil das
Installationsprogramm noch nicht digital signiert ist. Wähle in diesem Fall
**Weitere Informationen** und anschließend **Trotzdem ausführen**.

## So funktioniert es

Nach der Installation erscheint unten rechts neben der Windows-Uhr ein kleines
Fuchssymbol. Falls du es nicht siehst, klicke zuerst auf den Pfeil für ausgeblendete
Symbole.

1. Klicke mit der rechten Maustaste auf das Fuchssymbol.
2. Wähle deinen Lautsprecher aus der Liste aus.
3. Spiele auf dem PC Musik oder einen anderen Ton ab.

Zum Beenden der Übertragung wählst du im selben Menü **Gestoppt**. Unter
**Einstellungen** kannst du Lautstärke und Tastenkürzel ändern.

## Voraussetzungen

- Windows 10 oder Windows 11 in der 64-Bit-Version
- ein HomePod oder anderer AirPlay-2-Lautsprecher
- PC und Lautsprecher im selben privaten WLAN oder Heimnetzwerk

## Wenn kein Lautsprecher angezeigt wird

1. Prüfe, ob PC und Lautsprecher mit demselben Netzwerk verbunden sind.
2. Öffne das Fuchsmenü und wähle **Neu nach AirPlay-2-Geräten suchen**.
3. Prüfe in Windows, ob dein Netzwerk als **Privat** eingestellt ist.
4. Beende VulpiCast und starte es über das Windows-Startmenü erneut.

Falls das Problem bleibt, kannst du über **Protokollordner öffnen** die
Diagnosedatei finden und sie bei einer
[Fehlermeldung auf GitHub](https://github.com/TecTolik/VulpiCast/issues) anhängen.

## Deinstallation

Öffne **Windows-Einstellungen → Apps → Installierte Apps**, suche nach
**VulpiCast** und wähle **Deinstallieren**. Programmdateien, Verknüpfungen und die
Firewall-Regel werden automatisch entfernt.

## Datenschutz

Die Audioübertragung findet ausschließlich in deinem lokalen Netzwerk statt.
VulpiCast benötigt kein Benutzerkonto, enthält keine Werbung und sendet keine
Nutzungsdaten an externe Dienste.

## Open Source

VulpiCast ist freie Software unter der GPL-2.0-Lizenz. Das Projekt basiert auf
[HomePod Cast](https://github.com/iakacer/windows-airplay-homepod) und
[airplay2-rs](https://github.com/lmcgartland/airplay2-rs). Weitere Angaben stehen
in der Datei [LICENSE](LICENSE).

Apple, AirPlay, HomePod und Windows sind Marken ihrer jeweiligen Inhaber. Dieses
Projekt ist nicht mit Apple oder Microsoft verbunden.
