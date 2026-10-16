# ComfyUI backend — черновик спеки

Статус: зафиксированные решения (черновик)  
Референс клиента: `F:/python/brain/lib/comfyui.py`  
Связь с продуктом: `SPEC.md` §5.2 (AI-сценарий), §8 (AI workflow / backend boundary)  
Первый workflow: `comfyui/image.qwen_edit_21.json`

## 1. Цель

Сделать ComfyUI первым AI-backend редактора. Первый вертикальный срез — **Edit Image** через Qwen workflow:

```text
.env рядом с exe → HostConfig
→ взять raster входа (слой | документ) × scale
→ upload → пропатчить embedded workflow → POST /prompt
→ WS progress/preview + poll /history
→ скачать результат → показать финал в диалоге
→ Accept → новый слой в активном документе → закрыть диалог
  или Regenerate → новый запуск с новым случайным seed
```

Cancel в любой момент → `interrupt` на хосте + остановка локальной джобы.

## 2. Зафиксированные решения

| Тема | Решение |
|------|---------|
| Workflows | **Зашиваются в билд** (`include_str!` / embed из `comfyui/`). Рядом с exe их искать не нужно. |
| Первый workflow | `image.qwen_edit_21` (пользователь сам кладёт/правит json в `comfyui/`). |
| Потоки | **Отдельный OS-thread** + blocking HTTP (`reqwest` blocking). Без tokio runtime ради ComfyUI. |
| Отмена | Обязательна в MVP: UI Cancel → `POST …/interrupt` + флаг отмены джобы. |
| UI | Верхнее меню **File / Edit / AI** → **AI → Edit Image** открывает модальное диалоговое окно с настройками и ходом генерации. |
| Вход | Выбор: **текущий слой** или **весь документ** (flatten видимых). |
| Scale входа | Drag-поле, default `1.0`; рядом сразу показывать итоговый размер в px. Применяется только к входу. |
| Промпт | Текстовое поле → нода `Prompt` (`inputs.prompt`). |
| Вывод | Финальный результат остаётся в диалоге. **Accept** добавляет новый слой в активный документ и закрывает диалог; **Regenerate** запускает генерацию заново. |
| Seed | Каждый Run/Regenerate получает новый случайный числовой `Sampler.inputs.seed`. |
| Размер вывода | Финал показывается как вернул сервер. При Accept галочка **Fit Image** (default on) подгоняет результат точно под исходный размер входа до scale. Без галочки вставляется натуральный размер с обрезкой по холсту. Холст автоматически не расширяется. |
| Source=layer | Сырые пиксели активного слоя, без применения opacity слоя. |
| Negative prompt | Не требуется в MVP; остаётся пустым в workflow, поля в UI нет. |
| WebP | Поддержать декодирование результата ComfyUI, импорт и экспорт WebP в редакторе. |
| Работа во время генерации | Диалог остаётся открытым; переключение/закрытие документа и работа в фоне не предусмотрены. |

Прямое добавление результата слоем — осознанное упрощение первого среза относительно `SPEC.md` §5.2 и §8. Сохранение результата как ресурса/варианта проекта и provenance отложены.

## 3. Acceptance (MVP ComfyUI)

- `.env` рядом с exe (или cwd при `cargo run`): хост + optional Basic Auth.
- Workflow `image.qwen_edit_21` доступен из билда без внешних json на диске.
- В верхнем меню рядом с File и Edit расположен пункт AI; его команда Edit Image открывает модальный диалог: source (layer/document) → scale → итоговый WxH входа → prompt → Run.
- После Run в этом же диалоге доступны Cancel, прогресс и live-preview, если сервер их предоставляет. Окно продолжает отрисовываться и обрабатывать отмену.
- Cancel шлёт interrupt и не оставляет «вечный» busy в диалоге; фоновое продолжение генерации не предусмотрено.
- Успех: финальный результат заменяет live-preview в диалоге; документ пока не изменяется. Accept добавляет его на **новый слой** с учётом Fit Image, сохраняя размер холста, и закрывает диалог. Regenerate повторяет запуск на входе документа с новым случайным seed; непринятый результат не становится входом.
- Закрытие готового результата без Accept отбрасывает его, не меняя документ. При ошибке диалог остаётся открытым и показывает сообщение.
- Результат WebP декодируется; редактор также поддерживает импорт и экспорт WebP.
- Undo: добавление слоя участвует в обычной истории документа (как другие layer-ops), если это уже возможно без отдельной AI-истории.

## 4. Non-goals (сейчас)

- VastAI / RunPod / SSH / установка моделей и custom nodes.
- Визуальный редактор графа.
- Автоподстановка недостающих моделей.
- Видео/аудио outputs.
- Несколько параллельных jobs в UI.
- Фоновая генерация с закрытым диалогом и редактирование/переключение документов во время запуска.
- Negative prompt в UI.
- Сканирование `workflows/` на диске рядом с exe.
- Результат как Library asset / provenance в mega-file (позже).
- Выбор произвольного workflow из списка (первый срез = одна операция Edit Image).

## 5. Конфиг

Файл: `{exe_dir}/.env` (fallback: cwd).

| Ключ | Пример | Обязателен |
|------|--------|------------|
| `COMFYUI_HOST` | `https://comfyui.example.com` или `http://127.0.0.1:8188` | да |
| `COMFYUI_LOGIN` | `user` | нет |
| `COMFYUI_PASSWORD` | `secret` | нет |
| `COMFYUI_WS` | `wss://comfyui.example.com/ws` | нет (из HOST: `http→ws`, `https→wss`, путь `/ws`) |

Правила:

- Пустой login/password → без Authorization.
- Иначе Basic Auth на HTTP и на WS handshake (как в `listen_host_events`).
- Секреты не в проект и не в логи целиком.
- `.env` в `.gitignore`; в репо можно позже добавить `.env.example`.

Зависимости (ориентир): `dotenvy`, `reqwest` (blocking + multipart), `tungstenite`/`tokio-tungstenite` не обязателен — для WS достаточно blocking/`std`-friendly crate в worker-thread. Уже есть `serde_json`, `image`.

У `image` сейчас включён только PNG. Требуется добавить WebP decode/encode и использовать эту поддержку для результата ComfyUI, обычного импорта и экспорта. Текущий workflow сохраняет результат в WebP.

## 6. Embedded workflows

```text
comfyui/
  image.qwen_edit_21.json    # API format, в репо и в билд
src/
  comfyui/                   # Rust-клиент
```

Сборка: `include_str!("../../comfyui/image.qwen_edit_21.json")` (или `include_dir` когда workflows станет много).

Формат — API prompt JSON: `node_id → { class_type, inputs, _meta }`.

Параметризация как в `load_workflow`:

1. Индекс `_meta.title` → node.
2. Аргументы `"Title.input_key"` → `inputs[input_key]`.

### 6.1. Контракт `image.qwen_edit_21`

По текущему файлу:

| Title | class_type | Что патчим из редактора |
|-------|------------|-------------------------|
| `LoadImage` | `LoadImage` | `image` ← filename после upload |
| `Prompt` | `TextEncodeQwenImage21` | `prompt` ← текст UI; `negative_prompt` остаётся пустым |
| `Sampler` | `KSampler (Efficient)` | `seed` ← новое случайное целое для каждого запуска (0..2^53−1); steps/cfg пока как в json |
| `SaveImage` | `ImageSaveWithFormat (maga)` | не трогаем; результат читаем из history outputs |

Входной raster готовит редактор (слой/документ × scale), грузит через `/upload/image`, подставляет имя в `LoadImage.image`.

Размер входа на хост = `round(src_w * scale)` × `round(src_h * scale)` (минимум 1×1). Scale default `1.0`. В UI рядом с drag-полем: например `768 × 1024` после scale.

Scale влияет только на отправляемый raster. Результат скачивается и показывается с фактическими W×H сервера. При старте сохраняется исходная область входа в координатах документа (позиция + размер до scale): сейчас это весь слой/документ с позицией (0, 0).

**Fit Image** включён по умолчанию и влияет только на вставку по Accept: результат ресайзится точно до W×H исходной области и размещается в её позиции. Например, слой 512×512 при scale=2 отправляется как 1024×1024, а вставка с Fit Image остаётся 512×512. Галочку можно изменить после генерации без повторного запроса к серверу. Без Fit Image пиксели вставляются в натуральном размере от позиции входа. Холст никогда не расширяется автоматически: избыток обрезается по его границам, оставшееся место нового слоя прозрачно.

Сохранённые границы входа предусматривают размер и позицию будущего selection; выбор selection как входа пока не реализуется.

Empty latent / switch в графе пока не трогаем из UI — поведение как в зашитом json (`Switch.switch = false` → путь от encode).

## 7. Клиентский слой (Rust)

```text
src/comfyui/
  config.rs      // .env → HostConfig
  client.rs      // blocking HTTP: upload, prompt, history, view, interrupt
  ws.rs          // progress + binary preview (в том же worker или рядом)
  workflow.rs    // load embedded JSON + patch by title
  job.rs         // оркестрация одного запуска в отдельном thread
```

### 7.1. HTTP

| Действие | Endpoint |
|----------|----------|
| Очередь | `POST /prompt` `{ "prompt": workflow }` → `prompt_id` |
| Результат | `GET /history/{prompt_id}` |
| Скачать | `GET /view?filename=&subfolder=&type=` |
| Upload | `POST /upload/image` multipart + `overwrite=true` |
| Стоп | `POST /api/interrupt` (сверить path на целевом хосте; fallback `/interrupt`) |

### 7.2. WebSocket

- Text: `progress` → fraction; `status` → `queue_remaining`.
- Binary: проверить тип legacy preview и длину сообщения → skip 8 byte header → image → preview RGBA. Поддерживаются PNG/JPEG/WebP.
- WS открывается до `/prompt`, общий `client_id` связывает запуск и WS; события сопоставляются с `prompt_id`.
- WS читается в отдельном потоке с read timeout; потеря WS не прерывает polling history.
- UI читает `Mutex`/`channel` snapshot: `{ message, fraction, preview? }`.

### 7.3. Поток джобы

Worker thread:

1. Получить уже подготовленные RGBA bytes + W×H (scale сделан на UI/main до старта или в начале worker — без разницы, главное не блокировать UI на сети).
2. Upload → patch workflow → `POST /prompt`.
3. Крутить poll history + читать WS progress; периодически проверять cancel flag.
4. На Cancel: `interrupt`, выйти с `Cancelled`.
5. На success: download output image(s), вернуть через channel `Done { rgba, w, h }`.
6. На error/timeout: `Failed(msg)`.

Main/UI thread: старт джобы, опрос событий, обновление диалога; создание слоя только по Accept. До принятия полный результат хранится отдельно от документа, а для показа используется уменьшенная копия.

Timeout: общий срок генерации — 60 минут; HTTP connect — 5 секунд, запрос — 30 секунд, cancel-запросы — по 5 секунд. Poll history раз в секунду, проверка флага отмены между запросами каждые 100 мс. WS — отдельный поток с read timeout 200 мс после handshake.

Без авто-install моделей: ошибка хоста → текст пользователю.

## 8. UI: модальный диалог Edit Image

В верхнюю строку меню рядом с **File** и **Edit** добавляется меню **AI**. Его пункт **Edit Image** открывает модальный диалог (одна операция на MVP):

```text
File   Edit   AI
              └── Edit Image
```

```text
Edit Image (modal)
    ├── Source: ( ) Current layer  ( ) Whole document
    ├── Scale:  [drag 1.00]   →  1234 × 567 px
    ├── Prompt: [multiline]
    ├── [✓] Fit Image (при вставке: исходный размер до scale)
    ├── [Run] → во время выполнения [Cancel]
    ├── Preview + Progress ████░░  42% (если доступны)
    ├── После завершения: финальный результат + [Accept] [Regenerate]
    └── Status / error line
```

Поведение:

- **Current layer** — сырые пиксели активного слоя; opacity слоя не применяется, композитинг с соседями не нужен. Альфа-канал самих пикселей сохраняется во входе.
- **Whole document** — flatten видимых слоёв в размер холста (как экспорт/превью документа).
- Scale применяется только к выбранному входу; UI сразу пересчитывает и показывает px. Fit Image отдельно управляет подгонкой результата при вставке до исходного размера входа.
- Run недоступен без хоста/промпта/валидного входа. После запуска настройки недоступны, вместо Run показывается Cancel.
- Во время выполнения диалог остаётся открытым и блокирует взаимодействие с документом, его переключение и закрытие. UI продолжает обрабатывать события и отрисовывать генерацию.
- Закрытие диалога через крестик/Escape во время выполнения обрабатывается как Cancel; оно не переводит генерацию в фон.
- Закрытие приложения во время выполнения сначала отменяет запуск, затем завершает приложение.
- После Done: заменить preview финальным результатом, показать Accept и Regenerate, оставить окно открытым. Документ и история пока не изменяются.
- Accept: `add layer` с результатом, подогнанным в исходную область при включённом Fit Image, иначе в натуральном размере. Холст не расширять. Имя слоя roughly `Edit` / `AI Edit` + короткий хвост; сделать слой активным и закрыть диалог после успешного добавления.
- Regenerate: заново подготовить выбранный вход из исходного документа и отправить workflow с новым случайным seed. Предыдущий непринятый результат удаляется из диалога; его пиксели не используются как вход. Prompt/source/scale можно изменить перед повторным запуском.
- Закрытие диалога с готовым результатом без Accept отбрасывает результат, не добавляя слой.
- При Failed показать ошибку в диалоге и вернуть возможность Run. При Cancelled убрать busy и вернуть возможность Run; поздний Done отменённого запуска не добавляет слой.
- Preview и прогресс во время job показываются в этом же диалоге, когда доступны. Ни preview, ни готовый результат не добавляются на холст до Accept; отсутствие preview/численного прогресса само по себе не считается ошибкой.

## 9. Встраивание в app

```text
UI Edit Image modal
  → prepare input RGBA (layer | flatten) × scale
  → spawn worker(ComfyJob { host, workflow_id, prompt, image })
  → poll AiEvent::{Progress, Preview, Done, Failed, Cancelled}
  → on Done: hold full result + display final image in modal
  → on Accept: fit to captured input bounds if enabled → add layer without canvas resize → close modal
  → on Regenerate: discard unaccepted result → start worker with fresh random Sampler.seed
  → on Cancel click: set cancel flag → worker sends interrupt (без сетевого вызова в UI-thread)
```

Backend не владеет документом: только bytes in / bytes out.

## 10. План внедрения

### Phase 0 — каркас

- [x] Зависимости: `reqwest` (blocking), `dotenvy`, WS-crate; embed `comfyui/image.qwen_edit_21.json`.
- [x] WebP decode/encode: результат ComfyUI, импорт и экспорт в редакторе.
- [x] `HostConfig::from_env()`, проверка доступа к хосту.
- [x] `.env.example` без секретов.

### Phase 1 — клиент в worker-thread

- [x] upload / prompt / history / view / interrupt.
- [x] patch by `_meta.title` для LoadImage + Prompt.
- [x] Новый случайный `Sampler.seed` при каждом Run/Regenerate.
- [x] WS progress + preview decode.
- [x] Cancel path: flag + удаление своего задания из очереди + interrupt.
- [x] Прогон против живого хоста: результат WebP, preview/progress и отмена подтверждены отдельными live-тестами.

### Phase 2 — UI Edit Image

- [x] Верхнее меню File / Edit / AI → AI → Edit Image.
- [x] Модальный Edit Image: source, scale+px входа, prompt, Run → Cancel, progress/preview при наличии.
- [x] Подготовка входа layer/document; для layer — сырые пиксели без opacity.
- [x] Done → финальный результат в диалоге → Accept / Regenerate.
- [x] Accept → Fit Image по сохранённым исходным границам входа или натуральный размер → новый слой без изменения холста → закрытие диалога; закрытие без Accept не меняет документ.
- [x] Блокировка взаимодействия с документом на время диалога; закрытие во время генерации отменяет запуск.

Внешний вид и ручной UI-прогон проверяет пользователь. Автоматические проверки покрывают ошибки сервера, отмену и поздний результат, WebP, геометрию результата, добавление слоя и блокировку ввода документа.

### Phase 3 — позже

- [ ] Другие embedded workflows / список операций.
- [ ] Negative prompt, seed UI.
- [ ] Результат ещё и в assets + provenance в проекте.
- [ ] Sidecar meta для произвольных полей UI.

## 11. Риски

| Риск | Решение |
|------|---------|
| Path interrupt отличается | Проверить на целевом хосте; тонкий fallback |
| Binary preview header | Как в python: `message[8..]` |
| Flatten документа дорогой | Делать один раз при Run, не каждый кадр UI |
| Scale даёт огромный вход | Только конечный положительный scale; максимум 16384 px по стороне и 64 MP на входе |
| Undo нового слоя | В текущей истории есть только пиксельные правки. Добавление AI-слоя, как и импорт слоя, очищает историю; структурный undo остаётся общей задачей редактора |

## 12. Открытые вопросы

1. Нужен ли fit-to-max-side в дополнение к равномерному scale? Пока UI описан с одним float.
2. Старые серверы могут игнорировать `prompt_id` у interrupt: перед вызовом проверяем, что выполняется наше задание; для общего хоста с конкурентными клиентами потребуется серверная атомарная адресная отмена.

## 13. Критерий стопа

Phase 2: Edit Image открывает модальный диалог; с живым `.env` картинка правится через `image.qwen_edit_21`. В диалоге видны preview/прогресс, если доступны, и работает Cancel. После генерации показан финальный результат, включая WebP. Accept добавляет новый слой в активный документ с учётом Fit Image, сохраняя размер холста, и закрывает диалог; Regenerate повторяет генерацию с новым случайным seed. Закрытие без принятия не меняет документ. Импорт и экспорт WebP доступны в редакторе.
