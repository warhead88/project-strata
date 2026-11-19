# Рельеф, гидрология, биомы и геология

**Статус.** Принятая архитектура подсистемы физико-географического мира (`strata-terrain`) и её асинхронной визуализации в Bevy-клиенте (`strata-client`). Дополняет [Архитектуру глобуса](globe.md) и реализует требования к рельефу, климату, воде, подземным слоям и инженерной оценке местности.

## Граница между крейтами

- **`crates/strata-globe`** — координатный и геометрический фундамент (`PlanetSpec`, `EcefMeters`, `TileId`, `LocalOrigin`), контракт высот `ElevationSampler` / `ElevationRange`, выбор LOD и сшивка сеток тайлов.
- **`crates/strata-terrain`** — чистая `std`-библиотека (зависит только от `strata-globe`, без Bevy, окна и GPU). Владеет физико-географической моделью планеты (`TerrainWorld`): макро-кэшем высот и границ `min/max`, глобальным океаном, озёрами и реками, климатом и 14 биомами, геологическими слоями и минеральным потенциалом, локальными патчами рельефа и подземными пустотами, а также API оценки строительных площадок (`SiteAssessment`).
- **`crates/strata-client`** — Bevy-клиент. Хранит `TerrainElevation` и адаптивный рейкаст ([terrain.rs](../crates/strata-client/src/terrain.rs)), строит цветные меши земли и сшитые меши воды за один проход в локальных координатах тайла ([tile_mesh_build.rs](../crates/strata-client/src/tile_mesh_build.rs), [tile_mesh_colors.rs](../crates/strata-client/src/tile_mesh_colors.rs)), управляет асинхронной очередью с отменой и LRU-кэшем на GPU ([tile_streaming.rs](../crates/strata-client/src/tile_streaming.rs), [tile_async_queue.rs](../crates/strata-client/src/tile_async_queue.rs), [tile_cache.rs](../crates/strata-client/src/tile_cache.rs), [globe_render.rs](../crates/strata-client/src/globe_render.rs), [globe_materials.rs](../crates/strata-client/src/globe_materials.rs)), отображает отладочную сетку по клавише `G` ([globe_debug.rs](../crates/strata-client/src/globe_debug.rs)) и телеметрию в HUD ([markers.rs](../crates/strata-client/src/markers.rs), [hud.rs](../crates/strata-client/src/hud.rs)).

## Источники данных и `TerrainConfig`

Конструктор `TerrainWorld::new(config: TerrainConfig)` ([terrain_config.rs](../crates/strata-terrain/src/terrain_config.rs), [terrain_world.rs](../crates/strata-terrain/src/terrain_world.rs)) поддерживает два реальных источника `TerrainSource` (при произвольном `PlanetSpec` и настраиваемом `macro_resolution` степени двойки в `16..=512`, по умолчанию `DEFAULT_MACRO_RESOLUTION = 128`):

| Вариант `TerrainSource` | Назначение |
|---|---|
| `Earth { seed }` | Опорная физическая география Земли (`TerrainWorld::earth_default()`) на базе встроенного 15-минутного DEM SRTM15+ ([earth_relief_15m.sdem](../crates/strata-terrain/data/earth_relief_15m.sdem), [earth_dem.rs](../crates/strata-terrain/src/earth_dem.rs)): реальные континенты, шельфы, океанические котловины, 7 крупных озёр/внутренних морей и 15 крупнейших речных систем. |
| `Dem { grid: DemGrid, seed }` | Импорт внешней равнопромежуточной сетки высот `DemGrid` с пересчётом континентальности, климата и геологических провинций. |

## Макро-кэш и точные границы тайлов (`O(1)`)

`CubeMacroCache` ([macro_cache.rs](../crates/strata-terrain/src/macro_cache.rs)) запекает для каждой из 6 граней куба сетку `(N + 1) × (N + 1)` ячеек `MacroCell` (`macro_elevation_m`, `roughness`, `continentality`, `mean_temp_c`, `annual_precip_mm`, `geology_province`):

- **Бесшовное сэмплирование:** точка на сфере передаётся как единичный вектор `EcefMeters`. На границах граней соседние грани читают одинаковые мировые точки и получают побитово согласованный `MacroCell` без артефактов на швах и углах куба. Микро-детализация `micro_detail_fast` использует лёгкий 2-октавный 3D-шум, полностью отключаемый над океаном/равнинами и плавно затухающий у береговой линии (`0..40 м`), чтобы не искажать реальные контуры материков.
- **Мип-пирамида `min/max`:** поверх ячеек строится погранная иерархия `MinMaxNode`. Метод `ElevationSampler::tile_range(planet, tile)` за `O(1)` без аллокаций возвращает консервативный интервал высот тайла с учётом амплитуды микро-шума, глубины речных долин и активных патчей. Благодаря этому корректно работают выбор LOD, отсечение за горизонтом и быстрый отказ от водного меша на высокогорных сухих тайлах.

## Гидрология: океан, озёра и реки

Модуль [hydrology.rs](../crates/strata-terrain/src/hydrology.rs) и пространственный индекс [hydrology_index.rs](../crates/strata-terrain/src/hydrology_index.rs) используют бакеты уровня `LOD 4` (`6 × 16 × 16 = 1536` корзин) для ответа на `sample_water(unit_dir)` за `O(1)` без выделения памяти:

- **Глобальный океан:** уровень моря `SEA_LEVEL_METERS = 0.0`, 5 батиметрических зон `BathymetryZone` (`Abyssal`, `ContinentalSlope`, `ContinentalShelf`, `Littoral`, `Land`), а также предикат глубоководного порта `is_deepwater_port_depth` (`-200 .. -12 м`).
- **Озёра и внутренние моря:** для Земли заданы 7 водоёмов (`EARTH_LAKES` в [hydrology_data.rs](../crates/strata-terrain/src/hydrology_data.rs)): Каспийское море (`Saline`, `-28 м`), Байкал (`455 м`), Виктория (`1134 м`), Верхнее и Гурон-Мичиган, Танганьика, Титикака (`3812 м`).
- **Реки и речные долины:** 15 рек (`EARTH_RIVERS`: Амазонка, Нил, Миссисипи, Янцзы, Дунай, Волга, Рейн, Конго, Ганг, Меконг, Обь, Енисей, Лена, Парана, Муррей) задаются полилиниями `RiverWaypoint` с проверкой монотонно невозрастающего `surface_elevation_m` от истока к устью и классом судоходства `Navigability` (`SeaGoingEstuary`, `RiverVessel`, `Barge`, `Unnavigable`). Реки прорезают как широкую пойму/долину (`valley_half_width_m`), так и погружённое русло (`half_width_m`, `channel_depth_m`), чтобы водная гладь укладывалась в рельеф ниже берегов.

## Климат, биомы и геология

- **Климат и 14 биомов ([climate.rs](../crates/strata-terrain/src/climate.rs)):** `ClimateSample` учитывает высотный градиент температуры (`-6.5 °C / км`), индекс аридности, снежный покров и вероятность вечной мерзлоты (`permafrost_prob`). Классификатор `classify_biome` выделяет 14 биомов `Biome`: `PolarIce`, `Tundra`, `BorealForest`, `TemperateForest`, `TemperateGrassland`, `Steppe`, `HotDesert`, `ColdDesert`, `Savanna`, `TropicalRainforest`, `AlpineTundra`, `AlpineRock`, `CoastalWetland`, `OpenWater`. Функция `surface_albedo_rgb` вычисляет физико-географическое альбедо с учётом высоты, снега, уклона (выход скальных пород на крутых склонах) и типа покрова.
- **Геология и недра ([strata.rs](../crates/strata-terrain/src/strata.rs), [strata_derive.rs](../crates/strata-terrain/src/strata_derive.rs)):** `StrataColumn` описывает до `MAX_STRATA_LAYERS = 5` пластов до глубины `MAX_STRATA_DEPTH_M = 10_000.0` м. Хранит `GeologyProvince` (8 провинций: от `OceanicAbyss` и `ContinentalShelf` до `CratonShield`, `YoungOrogen`, `SubductionArc` и `PolarShield`), покровный грунт `SurfaceMaterial` (9 типов), литологию `BedrockKind` (7 типов пород), мощность рыхлого наноса, мерзлоту, глубину грунтовых вод, нормированные потенциалы 9 полезных ископаемых `MineralPotential`, стоимость выемки `excavation_cost_multiplier`, пригодность для подземного строительства `underground_suitability` и температуру пород `ambient_rock_temp_c`.

## Модификации рельефа и подземные пустоты

Слой патчей [patches.rs](../crates/strata-terrain/src/patches.rs) индексируется по бакетам `LOD 5` (при отсутствии патчей работает быстрый путь без накладных расходов). Поддерживаются 4 параметрических типа `TerrainPatchKind`:

- `FlattenPad` — выровненная строительная площадка с плавным переходным откосом (`blend_width_m`).
- `CorridorGrade` — линейно спрофилированный коридор между двумя точками (дороги, ж/д, ВПП).
- `ExcavationPit` — открытый карьер / котлован с понижением дна и бортами.
- `UndergroundVoid` — подземная выработка/комплекс (шахта, тоннель, бункер) в диапазоне глубин `top_depth_m..bottom_depth_m` с опциональным наземным порталом (`portal_surface_unit`). Доступность проверяется через `has_underground_void_at` / `has_any_underground_void`.

Каждая операция `apply_patch` и `remove_patch` инкрементирует `revision()` и возвращает `PatchReceipt` со списком затронутых тайлов `LOD 5` для точечной инвалидации клиентского кэша.

## Запросы симуляции и оценка площадок

Модуль [query.rs](../crates/strata-terrain/src/query.rs) и [site_assessment.rs](../crates/strata-terrain/src/site_assessment.rs) предоставляют:

- `TerrainWorld::sample_terrain(unit_dir) -> TerrainSample` — полный снимок точки (высота, уклон `slope_rad`/`slope_percent`, экспозиция `aspect_rad`, вода, батиметрия, климат, биом, провинция, поверхностный материал, мерзлота, наличие подземной пустоты).
- `TerrainWorld::sample_tile_vertex_base(unit_dir) -> TileVertexBase` — облегчённый zero-alloc однопроходный запрос для генератора мешей (`elevation_m`, `water`, `climate`, `biome`), исключающий повторное сэмплирование вершины.
- `TerrainWorld::assess_site(center_unit, radius_m) -> SiteAssessment` — агрегированная инженерно-географическая оценка по 25 точкам концентрических колец (перепад высот, доля суши и пригодных под застройку/ВПП площадей, наличие пресной воды, судоходных рек, глубоководного порта, сложность земляных и подземных работ, риск паводков и 12 битовых флагов `GeographicTags`).

## Клиентская визуализация и асинхронный стриминг

- **Тайловые координаты (`LocalOrigin`) и однопроходная сборка ([tile_mesh_build.rs](../crates/strata-client/src/tile_mesh_build.rs), [tile_mesh_colors.rs](../crates/strata-client/src/tile_mesh_colors.rs)):** `build_tile_meshes_cancellable` опрашивает `sample_tile_vertex_base` ровно один раз на каждую вершину сетки `(N+1)×(N+1)`. Внутренние нормали вычисляются центральными разностями по уже готовым позициям ECEF без лишних запросов к `TerrainWorld`, а нормали периметра — по шво-каноничному базису `canonical_tangent_basis` для побитового совпадения на границах тайлов и граней куба. Все вершины привязаны к `tile_origin = LocalOrigin::new(tile_center_ecef)` (перебазирование `OriginState` за `O(1)`).
- **Zero-rebuild сшивка рёбер (`StitchedIndexTable`):** вершины тайла не зависят от маски соседних LOD `CoarserEdges`. Таблица [tile_cache.rs](../crates/strata-client/src/tile_cache.rs) предвычисляет 16 буферов индексов (`0..16`); при изменении `coarser_edges` у кэшированного тайла `restitch_cached_tile` подменяет `Indices::U32` на месте в `Assets<Mesh>` без перезапуска фоновых задач.
- **Отрисовка суши, воды и адаптивный рейкаст:** суша и дно получают линейные RGBA-цвета `Mesh::ATTRIBUTE_COLOR` из кэшированных базовых выборок без дополнительных запросов. При наличии воды строится сшитый меш воды (`perceptual_roughness: 0.16`, `reflectance: 0.65`, подворот сухих береговых вершин на `tuck_m`). Курсорный рейкаст ([terrain.rs](../crates/strata-client/src/terrain.rs)) использует 48 грубых шагов и 24 шага бисекции (`<= 1e-6`).
- **Тихий фоновый стриминг и отсутствие чёрных дыр при отдалении ([tile_streaming.rs](../crates/strata-client/src/tile_streaming.rs), [tile_async_queue.rs](../crates/strata-client/src/tile_async_queue.rs), [globe_render.rs](../crates/strata-client/src/globe_render.rs)):** очередь ограничена `MAX_IN_FLIGHT_TASKS = 3`, `MAX_DISPATCH_PER_FRAME = 3`, `MAX_UPLOAD_PER_FRAME = 4` с флагом немедленной отмены `AtomicBool` для устаревших задач и приоритетом базовой оболочки (`lod <= min_lod`). Кэш `MAX_CACHED_TILES = 768` никогда не вытесняет видимые или базовые (`lod <= min_lod`) тайлы. При неизменном состоянии камеры/рельефа/origin/overlay `sync_globe_tiles` завершается за `O(1)` (idle fast path), а HUD ([hud.rs](../crates/strata-client/src/hud.rs), [markers.rs](../crates/strata-client/src/markers.rs)) обновляет `Text` только при изменении строки.

## Проверка и запуск

- Тесты библиотеки рельефа ([geography_and_climate.rs](../crates/strata-terrain/tests/geography_and_climate.rs), [hydrology_and_modes.rs](../crates/strata-terrain/tests/hydrology_and_modes.rs), [seams_and_bounds.rs](../crates/strata-terrain/tests/seams_and_bounds.rs), [strata_patches_and_queries.rs](../crates/strata-terrain/tests/strata_patches_and_queries.rs)): `cargo test -p strata-terrain`
- Тесты клиентского рендера, однопроходной сшивки швов, zero-rebuild stitching, отмены задач и закреплённого LRU ([terrain_render_tests.rs](../crates/strata-client/src/terrain_render_tests.rs), [terrain_streaming_tests.rs](../crates/strata-client/src/terrain_streaming_tests.rs), [camera_tests.rs](../crates/strata-client/src/camera_tests.rs)): `cargo test -p strata-client`
- Полная проверка лимитов файлов, форматирования, Clippy и тестов: `cargo verify`
- Запуск 3D-клиента с рельефом Земли: `cargo run --locked`
