# Linux Console: Детальный дизайн (TUI слой)

## 1. Scope и цели

Этот документ описывает дизайн Linux-консоли внутри TUI:
- UX и поведение ввода/вывода.
- Архитектуру исполнения команд, подсказок, истории, алиасов.
- Безопасную работу с `sudo`.
- Интеграции с Portage/Gentoo и файловыми/dependency-tree утилитами.
- Разделение платформенной логики `windows/linux`.

Документ предназначен как база для реализации и тест-плана.

---

## 2. Product principles

1. Консоль не должна блокировать UI ни при каких командах.
2. Пользователь всегда понимает текущее состояние (`idle/running/error/interrupted`).
3. Ошибки не теряются и имеют понятную поверхность в интерфейсе.
4. Безопасность по умолчанию: никаких секретов в наших кэшах.
5. Подсказки должны быть быстрыми и контекстными.
6. Логика платформы изолирована адаптерами, UI и контракты общие.

---

## 3. UI/UX дизайн консоли

## 3.1 Layout (Console tab)

```
┌ Header: cwd | git branch | shell | user@host | sudo-state | profile(dev/stage/prod)
├ Output Pane (scrollback, stdout/stderr split color, markers for errors)
├ Inline Panels (contextual):
│  - Suggestions
│  - History fuzzy search
│  - Portage package hints
└ Input Line:
   [mode badge] [prompt] <editable buffer>                         [status badge]
```

`status badge` справа от строки ввода:
- `running` (spinner)
- `error` (краткая причина/exit code)
- `interrupted`
- `stopped` (timeout/signal)

Правило показа:
- если команда выполнилась быстрее `status_threshold_ms`, бейдж не показывается;
- если дольше, показывается и держится `status_persist_ms` после завершения.

## 3.2 Console modes

- `Normal`
  - навигация по выводу, переходы между панелями.
- `Insert`
  - редактирование текущей команды (multiline).
- `HistorySearch`
  - fuzzy-поиск по истории с предпросмотром.
- `SuggestionSelect`
  - выбор подсказки для автодополнения/исправления.
- `Confirm`
  - подтверждение опасных действий и `sudo`-эскалации.

## 3.3 Key behavior

- Любой печатный символ в `Normal` переводит в `Insert`.
- `Enter`:
  - в `Insert`: execute/submit.
  - в `HistorySearch/SuggestionSelect`: принять выбор в buffer.
- `Esc`:
  - отмена panel mode;
  - при `running` может инициировать interrupt.
- `Ctrl+C`:
  - если задача активна: interrupt task;
  - иначе: поведение app-level (выход/подтверждение по глобальной политике).
- `Ctrl+R`: история (fuzzy reverse search).
- `Tab`: автодополнение/циклический выбор подсказок.

---

## 4. State machine и жизненный цикл команды

## 4.1 State machine (high level)

`Idle -> Validating -> AwaitConfirm? -> Spawning -> Running -> Completed | Failed | Interrupted | TimedOut`

Детали:
- `Validating`:
  - alias resolution, lint, risk scoring.
- `AwaitConfirm`:
  - для `sudo`, destructive команд, политики окружения.
- `Running`:
  - стрим stdout/stderr в output pane.
- `Completed/Failed/Interrupted/TimedOut`:
  - обновление status badge, запись в историю, telemetry.

## 4.2 Command execution data model

```rust
enum TaskState {
    Pending,
    Running { started_at_ms: u64 },
    Completed { exit_code: i32, elapsed_ms: u64 },
    Failed { reason: String, exit_code: Option<i32>, elapsed_ms: u64 },
    Interrupted { elapsed_ms: u64 },
    TimedOut { timeout_ms: u64 },
}

struct CommandTask {
    id: u64,
    input_raw: String,
    input_resolved: String,
    cwd: String,
    shell: String,
    uses_sudo: bool,
    started_at_ms: Option<u64>,
    finished_at_ms: Option<u64>,
    stdout_tail: Vec<String>,
    stderr_tail: Vec<String>,
    state: TaskState,
}
```

---

## 5. `sudo` UX и безопасность

## 5.1 Ключевая политика

- Не храним sudo-пароль в TUI/файлах/памяти приложения.
- Используем штатную модель `sudo`:
  - `sudo -n true` для проверки действующего timestamp.
  - `sudo -v` для обновления timestamp.
  - `sudo -k` для принудительного сброса (по запросу пользователя).

## 5.2 UX сценарий

1. Пользователь запускает команду, требующую root.
2. В `Confirm` показываем:
   - команда;
   - причина elevated mode;
   - кнопки: `Run with sudo` / `Run without sudo` / `Cancel`.
3. Если `sudo -n true` успешен:
   - запускаем сразу после подтверждения.
4. Если нет:
   - запускаем через PTY с системным prompt `sudo`.

## 5.3 Config knobs

```toml
[console.sudo]
confirm_on_privileged = true
auto_prepend = true
timestamp_check = true
never_store_password = true
```

---

## 6. Suggestions engine (команды, флаги, пакетный менеджер)

## 6.1 Provider architecture

Провайдеры подсказок работают параллельно, объединяются ранкером:
- `BuiltinProvider` (shell builtins)
- `PathBinaryProvider` (`$PATH`)
- `AliasProvider` (aliases.toml + runtime aliases)
- `HistoryProvider` (freq + recency)
- `PortageProvider` (emerge/equery/eix metadata)
- `CommandFlagProvider` (флаги текущей команды)

Общий контракт:

```rust
trait SuggestionProvider {
    fn suggest(&self, ctx: &SuggestContext) -> Vec<Suggestion>;
}
```

## 6.2 Ranking strategy

Score = `prefix_score * w1 + fuzzy_score * w2 + frequency * w3 + recency * w4 + context_boost * w5`

Базовый приоритет:
1. exact prefix
2. command+flag exact match
3. alias exact
4. fuzzy matches

## 6.3 Ошибки ввода

При неизвестной команде:
- `Did you mean: ...`
- hot actions:
  - `replace and run`
  - `replace in buffer`
  - `show package providing command` (для Gentoo)

---

## 7. Portage/Gentoo функционал

## 7.1 Минимальный функционал (P0/P1)

- Подсказки атомов: `category/package`.
- Подсказки флагов `emerge`.
- Быстрый просмотр:
  - installed?
  - available version(s)
  - USE flags
  - краткая metadata (description/homepage/license)
- Команды-обертки:
  - `pkg find <term>`
  - `pkg info <atom>`
  - `pkg uses <atom>`
  - `pkg deps <atom>`

## 7.2 Источники данных (приоритет)

1. Локальные данные Portage (главный источник, самый точный для системы).
2. CLI-инструменты:
   - `emerge`
   - `equery` (gentoolkit)
   - `eix` (если установлен).
3. Web fallback (`packages.gentoo.org`) для карточек и переходов в браузер.

## 7.3 Онлайн-режим и браузер

- Встроенная команда:
  - `pkg web <atom>` -> открыть страницу пакета в браузере.
- Встроенная команда:
  - `pkg search-web <term>` -> открыть результаты поиска.
- graceful fallback:
  - если сеть недоступна, показывать локальный summary.

---

## 8. Интеграции tree/dependency tooling

## 8.1 File tree

Поддержать внешние инструменты с автодетектом:
- `tree`
- `eza --tree`
- `fd` (быстрый индекс/поиск)
- `ncdu` (interactive disk usage)

## 8.2 Dependency tree

Для пакетов Gentoo:
- `equery depgraph <atom>` (или эквивалентный путь через Portage APIs).
- опционально `pacvis` для визуальных графов.

## 8.3 UI-представление

- компактный ASCII tree в output pane.
- folding для глубоких узлов.
- фильтры:
  - depth
  - include/exclude glob
  - size threshold

---

## 9. Everything integration strategy

Нативной Linux-версии Everything нет.

Поддерживаем 2 режима:
1. `native`:
   - `fd + rg + plocate` pipeline.
2. `remote-everything`:
   - bridge к удаленному Everything (Windows host, ETP/HTTP).

Config:

```toml
[console.search]
mode = "native" # native | remote_everything
remote_endpoint = ""
timeout_ms = 1200
```

---

## 10. Кроссплатформенное разделение (мониторы/executor)

## 10.1 Структура модулей

```
src/
  platform/
    mod.rs
    linux/
      monitors/
      executor/
      packages/
    windows/
      monitors/
      executor/
  console/
    mod.rs
    state.rs
    ui.rs
    history.rs
    suggestions.rs
    sudo.rs
```

## 10.2 Traits и фабрика

```rust
trait PlatformMonitors { /* ... */ }
trait CommandExecutor { /* ... */ }
trait PackageAdvisor { /* ... */ }

fn make_platform_services() -> PlatformServices { /* cfg(target_os) */ }
```

---

## 11. Конфиг: расширенная схема (draft)

```toml
[console]
shell = "bash"
history_limit = 100000
status_threshold_ms = 400
status_persist_ms = 1800
max_output_kb = 1024
multiline = true

[console.sudo]
confirm_on_privileged = true
auto_prepend = true
timestamp_check = true
never_store_password = true

[console.suggestions]
max_items = 12
fuzzy = true
provider_timeout_ms = 30
show_did_you_mean = true

[console.gentoo]
enable_portage_assist = true
prefer_local_metadata = true
use_eix_if_present = true
enable_web_fallback = true

[console.tree]
prefer = "eza" # eza | tree
max_depth = 6
```

---

## 12. Наблюдаемость и telemetry

- Метрики:
  - `command_exec_ms` (p50/p95/p99)
  - `suggest_latency_ms`
  - `history_search_ms`
  - `ui_frame_drops`
- События:
  - command_started/finished/interrupted
  - sudo_confirmed/denied
  - suggestion_applied
  - package_lookup_failed

---

## 13. Test strategy

## 13.1 Unit tests

- parser command-line
- alias cycle detection
- suggestion ranking
- status badge threshold behavior
- sudo policy checks

## 13.2 Integration tests

- async command stream no-UI-block
- command cancel/timeout
- hot-reload config apply/rollback
- Portage provider with mocked outputs

## 13.3 UX regression scenarios

- старт ввода с пустой строки
- ошибка команды видна в output + status badge
- команда < threshold не показывает статус
- команда > threshold показывает `running` и финальный state

---

## 14. Prioritized rollout

## P0 (критично)
- Асинхронный command runtime + reliable status badge справа от input.
- Console modes (`Normal/Insert/HistorySearch/SuggestionSelect`).
- Безопасный `sudo` flow без хранения пароля.
- Базовые подсказки: `$PATH + history + aliases`.

## P1
- Portage provider (atoms/flags/info/uses/deps).
- Tree/dependency integration (`tree/eza/equery`).
- Расширенный history index и ranking.

## P2
- Remote Everything bridge.
- recipes/macros и shared command packs.
- richer visual diff outputs/command cards.

---

## 15. Критерии приемки

1. UI остается отзывчивым при выполнении длительных команд.
2. Ошибка любой команды отображается в output pane и в статусе.
3. `sudo` работает через системный механизм timestamp без локального хранения пароля.
4. Подсказки команд и флагов Portage доступны в пределах целевого latency.
5. Пользователь может получить package summary и открыть web-страницу пакета из консоли.
6. Tree/dependency функции доступны как встроенные команды-коннекторы.
