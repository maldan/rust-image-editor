# Формат файла проекта (`.mgimg`)

Проект — один файл `.mgimg`: контейнер **MGFL** через `mega-file`.
Поверх него лежит схема приложения с виртуальными путями и JSON-манифестом.

Код: `src/workspace.rs` (схема и I/O), `src/safe_save.rs` (безопасная запись),
`src/project.rs` (модель документа в памяти).

## Контейнер

`mega-file` даёт:

- виртуальные пути (`/manifest.json`, `/assets/...`);
- тип записи (`kind`);
- CRC-32C;
- пакетную запись;
- **append-only** обновления при обычном Save;
- компакцию через **Optimize** (`mega-file::optimize`).

Обычный Save в тот же путь дописывает только изменённые записи.
Save As / первое сохранение пишет новый архив целиком во временный файл и
атомарно подменяет целевой (на Windows — `ReplaceFileW`, чтобы открытые
читатели старого снимка не ломались).

## Дерево внутри архива

```text
/manifest.json                          схема workspace (JSON)
/assets/{id}.webp                       ресурс библиотеки (lossless WebP; старые PNG читаются)
/videos/{id}.mp4                        видеоресурс
/documents/{doc_id}/layers/{i}.tiles    растр слоя i (тайлы 64×64)
/documents/{doc_id}/layers/{i}.timeline held-drawings анимации (только type=animation)
/previews/assets/{id}.webp              миниатюра 84×84
/previews/videos/{id}.webp              постер видео (если есть)
/previews/documents/{id}.webp           миниатюра документа
/graphs/{id}.json                       шаблон графа (ссылки, без пикселей)
```

Индексы слоёв `{i}` — позиция в массиве `layers` манифеста (0-based).

## Манифест

Путь: `/manifest.json`  
Kind: `mega-image-editor/manifest+json`

```json
{
  "format": "mega-image-editor",
  "version": 10,
  "active_document": 0,
  "assets": [ /* AssetMeta */ ],
  "videos": [ /* VideoMeta */ ],
  "documents": [ /* DocumentMeta */ ],
  "graphs": [ /* GraphMeta */ ]
}
```

Пишется версия **10**. Читаются **5–10**. Старые файлы без `graphs` открываются с пустым списком (в UI остаётся Graph 1).

### AssetMeta

| поле | смысл |
|------|--------|
| `id`, `name` | идентификатор и имя в библиотеке |
| `width`, `height` | размер |
| `path` | путь к полному изображению |
| `preview` | путь к миниатюре (опционально в старых файлах) |

### VideoMeta

| поле | смысл |
|------|--------|
| `id`, `name` | идентификатор и имя |
| `path` | обычно `/videos/{id}.mp4` |
| `preview` | постер WebP, если есть |

### DocumentMeta

| поле | смысл |
|------|--------|
| `type` | `"image"` (по умолчанию) или `"animation"` |
| `animation` | настройки таймлайна (fps, length, frame, camera, play_start/end); есть только у animation |
| `id`, `name` | документ |
| `width`, `height` | размер холста (видимая область) |
| `active_layer` | индекс активного слоя |
| `layers` | список `LayerMeta` |
| `preview` | миниатюра документа |

`type` и наличие `animation` должны совпадать. У animation у каждого слоя
обязан быть `timeline`.

### GraphMeta

| поле | смысл |
|------|--------|
| `id`, `name` | идентификатор (`g_1`) и имя в Library |
| `version` | схема json графа (сейчас 1); дублируется в корне файла |
| `path` | обычно `/graphs/{id}.json` |

Файл графа — шаблон: ноды (`kind` + `data`), линки, вид. Пиксели не хранятся,
только id документа/ассета. Слой в v1 не пишется.

### LayerMeta

| поле | смысл |
|------|--------|
| `z` | глубина, конечное число в `[-10, 10]` |
| `name`, `visible`, `opacity` | UI-свойства (`opacity` по умолчанию 1.0) |
| `aiMetadata` | необязательные метаданные ИИ: `prompt` и `model` |
| `path` | путь к `.tiles` |
| `compression` | `"none"` / отсутствует, или `"zstd"` |
| `timeline` | путь к `.timeline` (только animation) |

Подробности растра и кодека — в [layers.md](layers.md).

## Что где лежит по смыслу

```text
Workspace (один .mgimg)
├── assets[]     → /assets + /previews/assets
├── videos[]     → /videos + /previews/videos
├── documents[]  → DocumentMeta в манифесте
│   ├── image      → только .tiles на слой
│   └── animation  → .tiles + .timeline на слой + Animation в манифесте
└── graphs[]     → /graphs/{id}.json
```

Метаданные документов и слоёв живут в манифесте; тяжёлые байты — в отдельных
записях архива. Undo/история правок в файл не пишется.

## Сохранение и загрузка

**Save (тот же путь):** сравнивает fingerprint/хэши с последним успешным
снимком (`SavedResources`). В архив уходят только изменившиеся assets,
layers, previews и манифест (если он изменился). Незагруженные
(deferred) записи копируются байт-в-байт без перекодирования.

**Save As / новый файл:** пишется полный набор записей; deferred-данные
читаются из прежнего снимка архива.

**Optimize:** компактит только уже сохранённую ревизию на диске.
Несохранённые правки в памяти не коммитит. Сырые слои (`compression`
отсутствует/`none`) сжимает в zstd и обновляет манифест.

**Open:** читает манифест и превью; полные пиксели документов/ассетов
остаются на диске (deferred) до активации документа или использования
ассета. Видео читается кусками из неизменяемого снимка архива.

Если на диске манифест изменился с момента последнего Save в эту сессию,
инкрементальный Save отказывается с ошибкой «Project was changed on disk».

## Версии схемы

| version | статус |
|---------|--------|
| 5, 6 | читаются; Optimize/Save могут сжать сырые слои и дописать previews |
| 7, 8, 9 | читаются |
| 10 | запись; graphs в манифесте |

Несовместимый `format` или `version` — отказ открыть файл.

## Связанные документы

- [layers.md](layers.md) — SparsePixels, `.tiles`, compression, lazy load
- [native-video.md](native-video.md) — чтение MP4 из архива и превью
- `SPEC.md` §9 — исходные намерения; фактическая схема — этот файл
