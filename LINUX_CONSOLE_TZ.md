# ТЗ: Модуль Linux Console и кроссплатформенная архитектура

Детальная проработка UX, архитектуры и rollout: `LINUX_CONSOLE_DESIGN.md`.

## 1. Цели
- Реализовать полноценный модуль Linux Console в рамках TUI: быстрый, понятный, удобный для повседневной работы.
- Разделить платформенную логику мониторинга на `windows/linux`.
- Разделить исполнение команд на `windows/linux` (PowerShell vs shell).
- Сохранить единый UX и общие контракты данных для UI.

## 2. Архитектурный подход
- Единое кроссплатформенное ядро TUI.
- Платформенные адаптеры выбираются через `cfg(target_os)` и фабрику провайдеров.

### 2.1 Core-контракты
- `MonitorService` trait:
  - `cpu`, `gpu`, `ram`, `disk`, `network`, `processes`, `services`.
- `CommandExecutor` trait:
  - `execute`
  - `execute_stream`
  - `suggest`
  - `validate`

### 2.2 Платформенные модули
- `platform/windows`
  - PowerShell/WMI/NVML адаптеры (текущая база).
- `platform/linux`
  - `/proc`, `/sys`, `sysfs`, `systemctl`, `nvidia-smi` (опционально).
  - Shell executor на `bash` (по умолчанию), расширяемо для `zsh/fish`.

## 3. Linux Console UX (MVP+)
- Режимы:
  - `Normal`
  - `Insert` (ввод команды)
  - `History Search` (fuzzy по истории)
  - `Suggestions` (подсказки команд/алиасов)
- Поведение:
  - старт ввода сразу по печатному символу
  - multiline ввод
  - отмена/прерывание (`Esc`, `Ctrl+C`)
  - scrollback и навигация по выводу
- Визуал:
  - отдельная панель `Console`
  - статус-строка: shell, cwd, latency, last exit code
  - явный статус состояния: `idle/running/error`

## 4. История, алиасы, подсказки

### 4.1 История
- Длинная персистентная история (`sqlite` или `jsonl`).
- Дедупликация + ранжирование по частоте/свежести.
- Fuzzy-поиск по истории.

### 4.2 Алиасы
- Пользовательские алиасы + файл `aliases.toml`.
- Неймспейсы: `sys.`, `proj.`, `user.`.
- Валидация и детект циклических alias-цепочек.

### 4.3 Подсказки
- Источники:
  - shell builtins
  - бинарники из `$PATH`
  - алиасы
  - история
- Ранжирование:
  - exact prefix > fuzzy score > frequency.
- При неверной команде:
  - `Did you mean ...` + быстрый выбор исправления.

## 5. Мониторы Windows/Linux
- Разделение по директориям:
  - `src/monitors/windows/*`
  - `src/monitors/linux/*`
- Общие типы данных оставить в `src/monitors/types.rs`.
- Для каждого монитора:
  - `supported/degraded/unavailable`
  - причина деградации и fallback-механика.

## 6. Command Executor Windows/Linux
- Windows: `PowerShellExecutor`.
- Linux: `ShellExecutor` (`bash -lc`, timeout, потоковый вывод).
- Общие требования:
  - timeout + cancellation
  - лимит вывода
  - безопасное логирование (sanitize)
  - без блокировок UI (async).

## 7. Конфиг и hot-reload
- Добавить секции:
  - `[platform.windows]`
  - `[platform.linux]`
  - `[console]` (shell, history_limit, suggestion_limit, alias_file, max_output_kb)
- Hot-reload:
  - применять без рестарта для `console/history/suggestions`
  - на битом конфиге: безопасный rollback к последней валидной версии.

## 8. Критерии качества
- UI не блокируется при длительных командах.
- P95 latency подсказок < 50ms.
- История 100k записей без заметных фризов.
- Smoke-проверки минимум на Ubuntu, Arch, Fedora.

## 9. План внедрения (этапы)
1. Вынести общие traits и фабрику платформенных провайдеров.
2. Разделить мониторы по `target_os`.
3. Добавить Linux `ShellExecutor` с async streaming.
4. Реализовать Linux Console UI (режимы, статус, scrollback).
5. Реализовать историю/алиасы/подсказки (fuzzy + ranking).
6. Добавить тесты, профилирование, UX-polish.
