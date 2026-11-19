# Рельеф, гидрология, биомы и геология

**Статус.** Принятая архитектура подсистемы физико-географического мира (`strata-terrain`) и её асинхронной визуализации в Bevy-клиенте (`strata-client`). Дополняет [Архитектуру глобуса](globe.md) и реализует требования к рельефу, климату, воде, подземным слоям и инженерной оценке местности.

## Граница между крейтами

- **`crates/strata-globe`** — координатный и геометрический фундамент (`PlanetSpec`, `EcefMeters`, `TileId`, `LocalOrigin`), контракт высот `ElevationSampler` / `ElevationRange`, выбор LOD и сшивка сеток тайлов.
- **`crates/strata-terrain`** — чистая `std`-библиотека (зависит только от `strata-globe`, без Bevy, окна и GPU). Владеет физико-географической моделью планеты (`TerrainWorld`): макро-кэшем высот и границ `min/max`, глобальным океаном, озёрами и реками, климатом и 14 биомами, геологическими слоями и минеральным потенциалом, локальными патчами рельефа и подземными пустотами, а также API оценки строительных площадок (`SiteAssessment`).
- **`crates/strata-client`** — Bevy-клиент. Хранит `TerrainElevation` ([terrain.rs](../crates/strata-client/src/terrain.rs)), строит цветные меши земли и сшитые меши воды в локальных координатах тайла ([tile_mesh_build.rs](../crates/strata-client/src/tile_mesh_build.rs)), выполняет асинхронный стриминг и LRU-кэширование на GPU ([tile_streaming.rs](../crates/strata-client/src/tile_streaming.rs), [tile_cache.rs](../crates/strata-client/src/tile_cache.rs), [globe_render.rs](../crates/strata-client/src/globe_render.rs)), отображает отладочную сетку по клавише `G` ([globe_debug.rs](../crates/strata-client/src/globe_debug.rs)) и телеметрию в HUD ([markers.rs](../crates/strata-client/src/markers.rs)).

## Источники данных и `TerrainConfig`

Конструктор `TerrainWorld::new(config: TerrainConfig)` ([terrain_config.rs](../crates/strata-terrain/src/terrain_config.rs), [terrain_world.rs](../crates/strata-terrain/src/terrain_world.rs)) поддерживает три режима `TerrainSource` (при произвольном `PlanetSpec` и настраиваемом `macro_resolution` степени двойки в `16..=512`, по умолчанию `DEFAULT_MACRO_RESOLUTION = 128`):

| Вариант `TerrainSource` | Назначение |
|---|---|
| `Earth { seed }` | Опорная физическая география Земли (`TerrainWorld::earth_default()`): континенты, горные пояса, шельфы, океанические котловины, 7 крупных озёр/внутренних морей и 15 крупнейших речных систем. |
| `Procedural { seed }` | Детерминированная генерация произвольной планеты по `seed` с теми же инвариантами высот, климата, гидрологии и геологии. |
| `Dem { grid: DemGrid, seed }` | Импорт внешней равнопромежуточной сетки высот `DemGrid` с пересчётом континентальности, климата и геологических провинций. |

## Макро-кэш и точные границы тайлов (`O(1)`)

`CubeMacroCache` ([macro_cache.rs](../crates/strata-terrain/src/macro_cache.rs)) запекает для каждой из 6 граней куба сетку `(N + 1) × (N + 1)` ячеек `MacroCell` (`macro_elevation_m`, `roughness`, `continentality`, `mean_temp_c`, `annual_precip_mm`, `geology_province`):

- **Бесшовное сэмплирование:** точка на сфере передаётся как единичный вектор `EcefMeters`. На границах граней соседние грани читают одинаковые мировые точки и получают побитово согласованный `MacroCell` без артефактов на швах и углах куба. Микро-детализация `micro_detail_offset` масштабируется от локальной шероховатости и зависит только от 3D-позиции `unit_dir` и `seed`.
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
- `TerrainWorld::sample_render_vertex(unit_dir, slope_sin) -> RenderVertexSample` — облегчённый однократный запрос для потоковой генерации мешей (`elevation_m`, `albedo_rgb`, `water`).
- `TerrainWorld::assess_site(center_unit, radius_m) -> SiteAssessment` — агрегированная инженерно-географическая оценка по 25 точкам концентрических колец (перепад высот, доля суши и пригодных под застройку/ВПП площадей, наличие пресной воды, судоходных рек, глубоководного порта, сложность земляных и подземных работ, риск паводков и 12 битовых флагов `GeographicTags`).

## Клиентская визуализация и асинхронный стриминг

- **Тайловые координаты (`LocalOrigin`) и сборка геометрии ([tile_mesh_build.rs](../crates/strata-client/src/tile_mesh_build.rs)):** `build_tile_meshes` строит вершины относительно собственного центра тайла `tile_origin = LocalOrigin::new(tile_center_ecef)`. Благодаря этому перебазирование `OriginState` при движении камеры обновляет только `Transform::translation = origin.to_render_vec3(tile_origin.ecef)` за `O(1)` — без перестройки единого меша.
- **Отрисовка суши и воды:** суша получает физико-географические цвета вершин `Mesh::ATTRIBUTE_COLOR` в линейном RGBA (включая естественную батиметрию морского дна, озёр и речных русел под водой). Если на тайле нет водных вершин (`water_mesh: None`), дополнительный draw call не создаётся. При наличии воды строится сшитый тем же `CoarserEdges` непрозрачный глянцевый меш воды (`perceptual_roughness: 0.16`, `reflectance: 0.65`, без артефактов альфа-сортировки), где сухие береговые вершины на пограничных тайлах подворачиваются под рельеф на `tuck_m = (cell_arc * 0.08).clamp(0.5, 45.0)` м, исключая зазоры и z-fighting от орбиты до `20 м`.
- **Асинхронная очередь и fallback ([tile_streaming.rs](../crates/strata-client/src/tile_streaming.rs), [tile_cache.rs](../crates/strata-client/src/tile_cache.rs)):** задачи `build_tile_meshes` запускаются в `AsyncComputeTaskPool` с бюджетами `MAX_DISPATCH_PER_FRAME = 12` (ближайшие к камере первыми) и `MAX_UPLOAD_PER_FRAME = 8`; устаревшие при смене LOD/ревизии задачи отбрасываются до загрузки. `TileStreamPlanner::compute_visible_tiles` удерживает готового предка видимым до готовности всех 4 потомков (и наоборот при отдалении), исключая дыры и Z-наложения. LRU-кэш ограничен `MAX_CACHED_TILES = 512` без вытеснения видимых тайлов.
- **HUD и отладочный режим (`G`):** клиент запускается сразу с `TerrainElevation::earth_default()`. В обычном режиме рендерятся цвета биомов, рельефа и воды; клавиша `G` переключает `GlobeRenderState::debug_overlay` (сетка граней куба и `Gizmos`-линии 12 рёбер). HUD ([markers.rs](../crates/strata-client/src/markers.rs)) выводит `Tiles / Cached / Pending`, `Debug (G): ON/OFF` и телеметрию точки фокуса (`Elev`, `Biome`, `Water`, `Province`).

## Проверка и запуск

- Тесты библиотеки рельефа ([geography_and_climate.rs](../crates/strata-terrain/tests/geography_and_climate.rs), [hydrology_and_modes.rs](../crates/strata-terrain/tests/hydrology_and_modes.rs), [seams_and_bounds.rs](../crates/strata-terrain/tests/seams_and_bounds.rs), [strata_patches_and_queries.rs](../crates/strata-terrain/tests/strata_patches_and_queries.rs)): `cargo test -p strata-terrain`
- Тесты клиентского рендера, стриминга, водных мешей и точности `LocalOrigin` ([terrain_render_tests.rs](../crates/strata-client/src/terrain_render_tests.rs), [camera_tests.rs](../crates/strata-client/src/camera_tests.rs)): `cargo test -p strata-client`
- Полная проверка лимитов файлов, форматирования, Clippy и тестов: `cargo verify`
- Запуск 3D-клиента с рельефом Земли: `cargo run --locked`
