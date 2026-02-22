# Linux Console: Дополнения и улучшения (v3)

> Документ-спутник к `LINUX_CONSOLE_DESIGN.md` и `LINUX_CONSOLE_TZ.md`.
> Цель: собрать идеи, вдохновлённые анализом болей линуксоидов и лучшими находками современных терминалов (Warp, Fish, Nushell, Atuin), которые органично ложатся в нашу архитектуру.
> **Фокус: только Linux.** Windows Console — *coming soon*.

---

## 1. Анализ болей: что бесит в обычном терминале

| # | Боль | Источник | Как ломается workflow |
|---|------|----------|-----------------------|
| 1 | **Слепое автодополнение** | Bash/Zsh | Tab ничего не показывает, пока не нажмёшь. Нет предпросмотра. |
| 2 | **Нечитаемые ошибки** | gcc, make, systemd | Стена красного текста, непонятно что делать. |
| 3 | **Забыл sudo** | повсеместно | `Permission denied` → `Up → Home → sudo → Enter`. Рутина. |
| 4 | **Путь не существует** | cd, cp, mv | Опечатка в пути обнаруживается только после Enter. |
| 5 | **Команда повисла — непонятно почему** | компиляция, wget | Нет таймера, нет индикации прогресса. Зависание или долгая работа? |
| 6 | **История — помойка** | .bash_history | Линейный файл, нет контекста (cwd, exit code, время), поиск убогий. |
| 7 | **Вывод команд — сплошная каша** | ls, find, log-файлы | Вывод одной команды сливается с выводом следующей. Нет группировки. |
| 8 | **Нет discoverability** | CLI в целом | Чтобы узнать флаги, надо читать man. Нет inline-подсказок. |
| 9 | **Multiline — мучение** | Bash | Обратный слеш или here-doc. Неудобно редактировать. |
| 10 | **Копирование вывода** | терминалы | Мышкой выделять текст из скроллбека — прошлый век. |

---

## 2. Предлагаемые улучшения

### 2.1 🔮 Fish-style Ghost Text (inline preview)

**Боль #1, #8**

Пока пользователь печатает в `Insert` mode, лучшее совпадение из History/Alias/PATH отображается **серым полупрозрачным текстом** справа от курсора.

- `Right Arrow` или `Ctrl+F` — принять весь ghost text.
- `Alt+Right` — принять одно слово.
- Ghost text обновляется при каждом нажатии клавиши (debounce 30ms).
- Источники приоритезации: exact history prefix → alias → PATH binary → fuzzy history.

```
Пример:
> sys                          ← пользователь напечатал
> systemctl restart nginx      ← ghost text (серым, из истории)
  [→ принять]
```

**Реализация:** Дополнительное поле `ghost_text: Option<String>` в `ConsoleState`. Рендер в `console.rs` как `Span` с `Color::DarkGray` после курсора.

---

### 2.2 🎨 Real-time подсветка синтаксиса в строке ввода

**Боль #4, #8**

Прямо во время набора:

- **Первый токен** (команда): зелёный если существует в `$PATH`/aliases, красный если нет.
- **Пути**: зелёный если `tokio::fs::metadata` подтверждает существование, красный если нет.
- **Флаги**: серый/нейтральный (известные флаги можно подсвечивать).
- **Строки в кавычках**: жёлтый.

```
Пример:
> systemctl restart nginx       ← всё зелёное (команда и аргумент валидны)
> sysemctl restart nginx        ← "sysemctl" красным (опечатка!)
> cat /etc/fstab                ← путь зелёным (файл существует)
> cat /etc/fstub                ← путь красным (файла нет)
```

**Реализация:** Async `tokio::fs` проверка с debounce. Токенизатор в `console_state.rs` разбивает `input_buffer` на `Vec<InputToken { text, kind, valid }>`. Рендер рисует каждый токен своим цветом.

---

### 2.3 🧱 Command Blocks (a la Warp)

**Боль #7, #10**

Каждая команда + её вывод группируются в визуальный «блок»:

```
┌─ $ systemctl status nginx                    [✓ 0] [0.3s]
│  ● nginx.service - A high performance web server
│    Active: active (running) since ...
│    ...
└──────────────────────────────────────────────────────
┌─ $ cat /etc/does-not-exist                   [✗ 1] [0.0s]
│  cat: /etc/does-not-exist: No such file or directory
└──────────────────────────────────────────────────────
```

Преимущества:

- Визуальное разделение: понятно где кончился вывод одной команды и начался другой.
- Каждый блок хранит: `command`, `exit_code`, `elapsed`, `stdout`, `stderr`.
- **Горячие клавиши на блоке** (в `Normal` mode): `y` — скопировать вывод блока, `r` — перезапустить команду, `e` — объяснить ошибку (Ollama).

**Реализация:** Заменяем `VecDeque<ConsoleMessage>` на `Vec<CommandBlock>`. Каждый `CommandBlock` содержит `Vec<OutputLine>` и метаданные. Рендерим блоки с рамками ratatui.

---

### 2.4 🤖 Ollama Error Explainer

**Боль #2**

Если команда завершилась с `exit_code != 0` и stderr непустой, в статусе блока появляется подсказка:

```
┌─ $ make -j8                                  [✗ 2] [14.2s]
│  src/main.c:42:5: error: expected ';' before '}' token
│  ...
│  ───────────────────────────────────────────
│  [Ctrl+E: Explain Error with AI]
└──────────────────────────────────────────────────────
```

По `Ctrl+E`:

1. Берём последние N строк stderr.
2. Отправляем в локальную Ollama с системным промптом: *"Explain this error concisely. Suggest a fix."*
3. Ответ рендерим прямо под блоком в выделенной рамке.

**Реализация:** Используем уже существующий `OllamaClient`. Добавляем `AsyncUpdate::ErrorExplanation { block_id, text }`.

---

### 2.5 ⏱️ Live Stopwatch + Умный Status Badge

**Боль #5**

В `DESIGN.md` уже описан `status badge`, расширяем его:

- Во время `Running`: показываем **живой таймер** `[⟳ 00:14s]`, обновляется каждую секунду.
- Порог видимости: если команда заняла < `status_threshold_ms` (400ms по умолчанию), бейдж не появляется вообще.
- После завершения:
  - `[✓ 0]` — exit 0, зелёный.
  - `[✗ 1]` — exit != 0, красный.
  - `[⊘]` — interrupted (Ctrl+C).
  - `[⏱ timeout]` — превышен лимит.
- Бейдж держится `status_persist_ms` (1800ms) и затем плавно тускнеет.

**Реализация:** Поле `started_at: Option<Instant>` в `CommandTask`. Рендер вычисляет `elapsed` на каждом UI frame.

---

### 2.6 🔐 Intelligent Sudo Fallback

**Боль #3**

Расширяем дизайн `sudo` из `DESIGN.md`:

1. **Auto-detect permission failure:** Если `exit_code` = 126/1 и stderr содержит `Permission denied` / `Operation not permitted` / `Access denied`:
   - Под блоком появляется подсказка: `[Ctrl+S: Re-run with sudo]`
2. **По нажатию Ctrl+S:**
   - Если `sudo -n true` успешен → сразу перезапускаем с sudo.
   - Если нет → показываем `Confirm` panel (как в DESIGN.md §5.2).
3. **Чёрный список:** Некоторые команды никогда не предлагаем с sudo (например, `rm -rf /`). Конфигурируется.

```
┌─ $ systemctl restart nginx                   [✗ 1] [0.1s]
│  Failed to restart nginx.service: Access denied
│  ───────────────────────────────────────────
│  [Ctrl+S: Re-run with sudo]  [Ctrl+E: Explain]
└──────────────────────────────────────────────────────
```

---

### 2.7 📚 Atuin-inspired Rich History

**Боль #6**

Вместо линейного `.bash_history` — полноценная БД:

**Хранение (SQLite):**

| Поле | Описание |
|------|----------|
| `id` | auto-increment |
| `command` | текст команды |
| `cwd` | рабочая директория |
| `exit_code` | код возврата |
| `duration_ms` | длительность |
| `timestamp` | unix timestamp |
| `session_id` | ID сессии TUI |
| `hostname` | имя хоста |

**Поиск (Ctrl+R → HistorySearch mode):**

- Full-screen fuzzy search через всю историю.
- Фильтры: по текущей директории, по сессии, глобально.
- Preview: показывает exit code + duration рядом с каждой строкой.
- Ранжирование: `recency * 0.4 + frequency * 0.3 + prefix_match * 0.3`.

**Емкость:** 100k+ записей, индексы по `command`, `cwd`, `timestamp`.

---

### 2.8 📝 Комфортный Multiline ввод

**Боль #9**

- `Shift+Enter` или `\` в конце строки → перевод на новую строку внутри `Insert` mode.
- Поле ввода динамически расширяется до 5 строк (конфигурируемо).
- `Enter` — выполнить всю команду целиком.
- Подсветка синтаксиса работает и в multiline.
- Визуально: номера строк слева, как в IDE.

```
> for f in *.txt; do         │1
>   echo "Processing $f"     │2
>   wc -l "$f"               │3
> done                        │4
```

---

### 2.9 💡 Inline Flag Hints (Man-page на лету)

**Боль #8**

Когда пользователь набрал команду и ставит пробел + `-`, показываем popup с наиболее частыми флагами:

```
> tar -
  ┌──────────────────────────┐
  │ -x  extract              │
  │ -c  create               │
  │ -f  file                 │
  │ -v  verbose              │
  │ -z  gzip                 │
  │ -j  bzip2                │
  └──────────────────────────┘
```

**Источники данных (по приоритету):**

1. Встроенная база популярных команд (top 200 Linux бинарников, ~5KB JSON).
2. Парсинг `command --help` (кешируется).
3. `man` page parsing (fallback, тяжелый).

**Реализация:** `CommandFlagProvider` из `DESIGN.md` §6.1. Встроенный JSON-файл `flag_hints.json` поставляется с бинарником.

---

### 2.10 🔄 Smart Retry & Command Recipes

Добавляем мета-команды, которые работают поверх нашего executor'а:

| Команда | Действие |
|---------|----------|
| `!!` | Повторить последнюю команду |
| `sudo !!` | Повторить с sudo |
| `!$` | Подставить последний аргумент предыдущей команды |
| `retry` | Повторить последнюю failed-команду |
| `retry -n 3` | Повторить до 3 раз с паузой |
| `explain` | Объяснить последнюю ошибку через Ollama |

---

### 2.11 📊 Session Dashboard (при открытии Console tab)

Вместо пустого экрана при первом переключении на Console tab — краткий dashboard:

```
╭─ Console Session ──────────────────────────────────╮
│  Shell: bash 5.2.21     User: echoeins@gentoo-box  │
│  CWD:   ~/projects/tui  Git: main (3 ahead)        │
│  Uptime: 4d 7h          Load: 0.42 0.38 0.35       │
│  Last cmd: systemctl status nginx [✓ 0] 2m ago      │
│                                                      │
│  [i] Insert mode   [Ctrl+R] History   [Tab] Complete │
╰──────────────────────────────────────────────────────╯
```

Показывается только если output_history пуст. Пропадает при первой команде.

---

## 3. Декомпозиция: предлагаемые этапы реализации (только Linux)

### Этап 1: Фундамент (Core Runtime)

- `CommandBlock` модель данных (замена `ConsoleMessage`)
- `TaskState` machine (Pending → Running → Completed/Failed/Interrupted/TimedOut)
- Live stopwatch + status badges
- Правильный `Ctrl+C` interrupt через PTY signal
- Session dashboard при пустой истории

### Этап 2: Качественная история

- SQLite backend для истории
- Запись context-полей (cwd, exit code, duration, hostname)
- `Ctrl+R` → `HistorySearch` mode с fuzzy поиском
- Дедупликация + ранжирование

### Этап 3: Умный ввод

- Ghost text (Fish-style inline preview)
- Real-time подсветка синтаксиса (команда/путь валидация)
- Multiline input
- `!!`, `!$`, `sudo !!` макросы

### Этап 4: Suggestions Engine

- Provider architecture (`BuiltinProvider`, `PathBinaryProvider`, `AliasProvider`, `HistoryProvider`)
- Inline flag hints (JSON база + `--help` парсинг)
- `Did you mean...` при неизвестной команде
- `SuggestionSelect` mode UI

### Этап 5: Sudo & Security

- `sudo -n true` проверка timestamp
- `Ctrl+S` smart retry with sudo
- `Confirm` panel UI
- Чёрный список опасных команд

### Этап 6: Ollama Integration

- `Ctrl+E` error explainer
- `explain` метакоманда
- Системный промпт для контекстного объяснения ошибок

### Этап 7: Portage/Gentoo

- `PortageProvider` для подсказок пакетов
- `pkg info/find/uses/deps/web` метакоманды
- Dependency tree визуализация

### Этап 8: Integrations & Polish

- Tree integration (eza/tree/fd auto-detect)
- Search integration (fd + rg + plocate)
- Config hot-reload для console-секций
- Telemetry metrics

---

## 4. Вопросы к обсуждению

1. **Ghost text vs Popup suggestions:** В DESIGN.md описан `SuggestionSelect` mode с панелью. Ghost text — дополнение или замена? Предлагаю: ghost text для лучшего совпадения, Tab → popup для списка альтернатив. Согласны?

2. **Command Blocks vs плоский скроллбек:** Blocks дают чёткую группировку, но усложняют рендер при очень длинном выводе (тысячи строк). Ставить ли лимит строк на блок с `[Show full output]`?

3. **SQLite vs JSONL для истории:** SQLite мощнее для поиска и фильтрации, но добавляет зависимость. JSONL проще. В DESIGN.md и TZ упомянуты оба варианта. Моя рекомендация — SQLite (как в Atuin), потому что fuzzy search по 100k записей в JSONL будет тормозить.

4. **Flag hints JSON:** Готовы ли вы к тому, что я подготовлю базу флагов для ~200 самых частых Linux-команд? Это ~15-20KB embedded ресурс.

5. **Ollama availability:** Если Ollama не запущена при нажатии Ctrl+E, показывать ли сообщение `"Ollama not running. Start with: ollama serve"`, или тихо игнорировать?

6. **Приоритеты этапов:** Предложенная декомпозиция идёт от фундамента к польским фичам. Если хотите перетасовать приоритеты (например, Portage раньше Ollama integration), я подстроюсь.

---

## 5. Заметка: Windows Console

> **Coming soon.** Все UI-контракты, `ConsoleState`, `CommandBlock`, history schema и suggestion traits проектируются кроссплатформенно. Windows-реализация (`PowerShellExecutor`) будет подключена через те же трейты после стабилизации Linux-версии.
