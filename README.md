# siboot (Vladislav-VB)

Форк [FrizzleM/siboot](https://github.com/FrizzleM/siboot): ставит на iPhone, прямо из
[Open Minis](https://apps.apple.com/app/id6759188481), без компьютера, подписав твоим Apple
Account, **на выбор**:

- **SideInstaller** — как в оригинале;
- **SideStore** — nightly (именно он понимает pairing-файл iOS 27) или stable;
- **LiveContainer + SideStore** — копия, которую ставит SideInstaller (с проверкой SHA-256);
- **свой .ipa** — файл с iPhone (кнопка «Choose .ipa» на странице открывает «Файлы»), ссылка,
  ранее использованный файл, файл из папки, подключённой в Minis, или путь вручную.

После установки siboot отдаёт приложению pairing-файл туда, где оно его ищет (SideInstaller,
SideStore, LiveContainer, StikDebug, Feather и др. — как это делает SideInstaller).

Нужно: iOS 27+, Developer Mode, Wi‑Fi, LocalDevVPN и Open Minis 1.16+. В терминале Minis:

```sh
wget -qO- https://raw.githubusercontent.com/Vladislav-VB/siboot/main/minis/install.sh | sh
```

Без меню: `siboot --app sidestore-nightly`, `siboot --app livecontainer`,
`siboot --ipa /var/minis/mounts/<папка>/App.ipa` или `siboot --ipa https://…/App.ipa`.

Подробности — в [minis/README.md](minis/README.md#what-it-installs).

---

A fork of FrizzleM/siboot that asks what to install: SideInstaller, SideStore, LiveContainer +
SideStore, or an .ipa of your own, picked on the page from this iPhone's files. See
[minis/README.md](minis/README.md). iOS 27+ is required.
