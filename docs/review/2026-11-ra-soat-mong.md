# Rà soát móng Ferro: những gì phải làm ngay từ đầu

- **Ngày:** 2026-11-23
- **Trạng thái:** Đề xuất. Chờ chủ dự án duyệt; mỗi mục được duyệt sẽ thành một ADR (tiếng Anh) và thay thế các `D-xx` liên quan.
- **Phạm vi:** đối chiếu 75 quyết định đã chốt (`docs/foundation/README.md`) với cách các hệ thống lớn đang làm, tìm chỗ nào phải sửa *trước khi* xây thêm. Được phép đề xuất đập cả quyết định đã chốt.

## 0. Tóm tắt

Phần lớn móng hiện tại **đúng hướng và nên giữ**: mm và radian, `Point3` ≠ `Vec3`, Command / Transaction, tài liệu bất biến chia sẻ cấu trúc (D-24), id topo bền vững (D-25), AI đề xuất thay vì tự sửa (D-26), IFC và Revit link, WASM, MPL.

Nhưng có **6 chỗ phải đổi ngay**. Làm muộn thì mỗi tính năng xây lên trên sẽ phải viết lại:

| # | Đổi gì | Đập quyết định nào | Vì sao phải làm ngay |
|---|---|---|---|
| F1 | **Element = ngữ nghĩa + nhiều biểu diễn hình học.** Category, Type, tham số tách khỏi *nguồn* hình học (tham số, Form tự do, mesh, scan). Mọi category đều có thể vẽ tự do. | D-12 (giữ ý), D-38, D-41 (`convert_form_to_walls`), D-70, `ElementBody` hiện tại | Đây là cái "tường vẽ kiểu gì cũng được mà vẫn có data". Mọi exporter, renderer, phân tích đều đọc chỗ này. |
| F2 | **Tham số có kiểu đại lượng vật lý** (spec như Revit `ForgeTypeId`, measure như IFC), đơn vị SI cho mọi thứ trừ chiều dài. Mỗi định nghĩa tham số có `Uid` như shared parameter của Revit. | Cách cài D-15, D-36; bổ sung D-01 | Năng lượng, kết cấu, carbon, dự toán đều cần. Đổi kiểu dữ liệu sau khi đã có file người dùng là đau nhất. |
| F3 | **Vật liệu = nhiều asset**: hiển thị (OpenPBR / MaterialX), vật lý, nhiệt, âm, môi trường (EPD), chi phí. | `Material { color, roughness }` | Lớp tường phải tự ra U-value, khối lượng, carbon. Render cũng cần PBR chuẩn. |
| F4 | **Hình học sẵn sàng NURBS và non-manifold** cho Form: body loại Solid / Sheet / Wire / General; mặt Plane, Cylinder, Cone, Sphere, Torus, NURBS. | D-11 (chỉ solid manifold), D-39 (chỉ Plane, Cylinder) | Bo cạnh, mái cong, vẽ tự do kiểu SketchUp (cạnh rời, mặt hở) đều cần. Đổi cấu trúc topo về sau là viết lại kernel. |
| F5 | **Quan hệ là dữ liệu hạng nhất**: join, cut (void), host, attach, bound (bao phòng), space boundary, connect. Cắt không phá hủy. | Mở rộng D-18, D-41 | Void, tường cắt sàn, phòng tự tính, mô hình năng lượng đều đứng trên đồ thị quan hệ. |
| F6 | **Thế giới chạy (Scene) tách khỏi Document**: Document là sự thật; Scene là dữ liệu dẫn xuất (ECS) cho render, vật lý, phân tích, có instancing, LOD, streaming. | Thêm vào D-48; thay `SceneCache` trong app | Ngân sách hiệu năng D-62 (100k element, 30 fps) không đạt được nếu mỗi hệ thống tự đọc Document. |

Ngoài ra có **6 việc móng nên quyết ngay, làm dần**: F7 dữ liệu thực tế (point cloud, splat), F8 phòng và mô hình năng lượng, F9 thời gian, mô phỏng và kết quả phân tích, F10 lớp ý kiến kiểu USD cho option và cộng tác, F11 lệnh và bộ phím, F12 đa ngôn ngữ, báo lỗi và kiểm tra (ADR 0003, đã duyệt).

**Thứ tự đề xuất:** Milestone 5 = "Móng 2" gồm F1, F2, F3, F5 (mô hình dữ liệu), rồi F4 (hình học), rồi F6 (Scene). Phần còn lại của M4 (reference line, dimension, sơn từng mặt, component) làm *trên* móng mới.

---

## 1. Đã đối chiếu với những hệ thống nào

| Nhóm | Hệ thống | Lấy bài học gì |
|---|---|---|
| BIM | Revit, ArchiCAD, Vectorworks, Autodesk AEC Data Model (cloud), Speckle | Category / Type / Instance, tham số và shared parameter, vật liệu nhiều asset, quan hệ join/cut, dữ liệu dạng hạt (granular) |
| Dựng hình | SketchUp, Rhino, Plasticity, Fusion, Onshape, Parasolid, OCCT, truck / monstertruck (Rust) | Sửa trực tiếp và lịch sử, body non-manifold, NURBS, dung sai |
| Thủ tục | Grasshopper, Dynamo, Houdini, Unreal PCG | Thuộc tính trên hình học, đồ thị phụ thuộc, tài sản số (HDA) |
| Game, render | Unreal Engine (Nanite, Lumen, World Partition, Datasmith), Twinmotion, Enscape, D5, Blender, Omniverse, OpenUSD (Core Spec 1.0, 12/2025) | Tách editor và runtime, ECS, instancing, streaming, USD composition, MaterialX / OpenPBR |
| Năng lượng, ánh sáng | EnergyPlus, OpenStudio, Ladybug / Honeybee, Radiance, gbXML, IES VE | Room = vùng kín, space boundary, construction có tham số nhiệt, vị trí địa lý và thời tiết |
| Kết cấu, vật lý | Revit analytical model, IFC structural analysis, Rapier, Jolt, Chaos | Biểu diễn phân tích riêng, collider dẫn xuất, thời gian mô phỏng |
| Dữ liệu thực tế | ReCap, E57, LAS / LAZ, COPC, Potree, 3D Tiles, Gaussian splat (glTF `KHR_gaussian_splatting`, bản release candidate 02/2026) | Octree streaming ngoài lõi, georeference, bắt điểm vào point cloud, scan-to-BIM |
| Trao đổi | IFC 4.3, IFC5 (alpha, JSON, ghép đối tượng theo lớp), USD, glTF, DWG / DXF, 3DM, SKP, BCF | Đối tượng ghép từ thành phần, lớp ghi đè, ánh xạ ổn định |
| Cộng tác | Figma, Onshape, Speckle, CRDT (Loro, Automerge) | Thay đổi theo thuộc tính, id ổn định, nhánh và merge |

---

## 2. F1: Element = ngữ nghĩa + biểu diễn

### Vấn đề

`ElementBody` hiện là một enum trộn hai việc: *nó là gì* (Wall, Floor) và *hình của nó đến từ đâu* (Wall thì sinh từ đường định vị và type; Form thì là B-rep tự do). Vì vậy tường chỉ có thể là tường tham số. Muốn vẽ tường bo cong thì phải là Form rồi "chuyển thành tường", và mất hình tự do.

Revit cũng mắc đúng lỗi này. Model in-place là lối thoát, nhưng công cụ dựng khối trong đó rất kém, và **Revit API không tạo được in-place family** (Autodesk xác nhận trên diễn đàn API).

IFC thì làm đúng: một `IfcWall` có thể mang nhiều *representation* (Axis, Body, FootPrint, Box), và Body có thể là extrusion tham số hay B-rep tùy ý. Unreal, USD và Houdini cũng tách "thực thể" khỏi "dữ liệu hình".

### Đề xuất

```text
Element
  ├─ ngữ nghĩa:  category, type, tham số, level, host, phase, option, tags
  ├─ geometry_source (một trong):
  │    Parametric(generator)   tường theo location line + CompoundStructure, sàn theo profile...
  │    Direct(Form body)       B-rep sửa trực tiếp kiểu SketchUp
  │    Mesh(asset)             mesh nhập về, không sửa topo
  │    Reality(asset, region)  đoạn point cloud hoặc splat được gán nghĩa
  └─ representations (dẫn xuất khi regenerate, có cache):
       Body (Coarse / Medium / Fine, D-69), Axis, Footprint, Analytical (kết cấu),
       EnergySurfaces, Clearance, Collider
```

Quy tắc:

1. **Mọi category chấp nhận mọi nguồn hình**, trừ datum (Level, Grid). Tường vẽ tự do = `Wall` + `Direct(Form)`. Nó vẫn có type, vật liệu, tham số; diện tích, thể tích, mặt ngoài và mặt trong được tính từ khối thật.
2. **Chuyển hai chiều không mất mát khi có thể**: `Parametric → Direct` luôn được (đóng băng hình). `Direct → Parametric` khi hình nhận ra được là tường thẳng hoặc cung, nếu không thì giữ Direct.
3. Semantic push-pull (D-73): trên Parametric thì đổi tham số; trên Direct thì sửa B-rep trực tiếp. Không còn hộp thoại "tạo void hay chuyển thành Form".
4. **Xuất Revit theo nguồn**: Parametric → tường, sàn Revit thật (như hiện nay). Direct hoặc Mesh → `DirectShape` đúng category kèm tham số (mặc định), hoặc tùy chọn sinh *loadable family* đúng category rồi đặt instance. In-place không làm được qua API.
5. **Xuất IFC**: đúng class (`IfcWall`...) với Body là `IfcExtrudedAreaSolid` (Parametric) hoặc `IfcAdvancedBrep` / `IfcFacetedBrep` (Direct).

### Đập gì

- `ElementBody::{Wall, Floor, Form...}` → tách thành `semantics` + `geometry_source`.
- D-41 `convert_form_to_walls` → thay bằng lệnh `set_category` và `set_geometry_source`.
- D-70: "Form / component" không còn là một *loại family*, mà là một *nguồn hình* dùng được cho mọi family.

---

## 3. F2: Tham số có kiểu đại lượng và đơn vị

### Vấn đề

`ParamValue` có `Length`, `Area`, `Volume`, `Number`... Không có hệ số dẫn nhiệt, U-value, khối lượng riêng, lực, tiền, kgCO2e. Một `Number` không biết nó là gì, nên không đổi đơn vị được, không kiểm tra được, và không map được sang IFC hay Revit.

### Đề xuất

1. `ParamValue::Quantity { spec, value }`, trong đó `spec` là id ổn định trong một bảng đại lượng: length, area, volume, angle, mass, density, force, moment, stress, thermal_conductivity, thermal_resistance, u_value, heat_capacity, temperature, power, illuminance, luminous_efficacy, flow, pressure, currency, carbon_mass, time, ratio...
2. Lưu SI chuẩn (W/m·K, kg/m³, N). **Chiều dài vẫn là mm** (D-01), góc là rad (D-02). Đổi đơn vị chỉ ở UI, MCP và file (giống luật hiện tại).
3. Bảng spec map 1:1 sang Revit `SpecTypeId` và IFC `Ifc*Measure`; tham khảo QUDT cho định nghĩa.
4. **Định nghĩa tham số là dữ liệu có `Uid`**, giống shared parameter GUID của Revit: key, spec, nhóm, instance hay type, mô tả, ánh xạ Revit và IFC Pset. Round-trip Revit hai chiều dựa vào `Uid` này chứ không dựa vào tên.

---

## 4. F3: Vật liệu nhiều asset

Revit tách Material thành Appearance, Physical (kết cấu) và Thermal asset. Unreal, Blender, Omniverse, Maya đang hội tụ về **OpenPBR**, trên nền **MaterialX**.

Đề xuất `Material` gồm:

| Asset | Nội dung | Dùng cho |
|---|---|---|
| `appearance` | OpenPBR (base, specular, transmission, coat, emission, normal), map texture có quy tắc ánh xạ | Render, xuất USD / glTF / Unreal |
| `physical` | ρ, E, ν, cường độ, hệ số giãn nở | Kết cấu, khối lượng |
| `thermal` | λ, c, ρ, độ phát xạ, độ hấp thụ mặt trời, kháng ẩm | Năng lượng, U-value tự động |
| `acoustic` | Hệ số hấp thụ theo dải tần | Âm học |
| `environmental` | GWP (A1–A3...), nguồn EPD | Carbon |
| `cost` | Đơn giá, đơn vị đo | Dự toán |

`CompoundStructure` đọc các asset này, nên **mỗi type tường hay sàn tự ra U-value, khối lượng trên m² và carbon trên m²**, không cần plugin.

---

## 5. F4: Hình học sẵn sàng NURBS và non-manifold

### Vấn đề

- D-11 bắt buộc solid kín, manifold. SketchUp cho phép cạnh rời, mặt hở, ba mặt chung một cạnh. Người dùng vẽ kiểu SketchUp sẽ chạm giới hạn này liên tục. Body hiện tại đã cho phép mặt hở nhưng vẫn cấm non-manifold.
- D-39 chỉ có Plane và Cylinder. Bo cạnh (fillet) cần Torus và NURBS; mái cong, vỏ tự do cần NURBS.

### Đề xuất

1. **Loại body như Parasolid**: `Solid`, `Sheet` (mặt hở), `Wire` (chỉ cạnh), `General` (non-manifold). Form mặc định là `General`; xuất ra ngoài thì kiểm tra và tách thành solid khi cần. Element xây dựng (tường, sàn) yêu cầu `Solid` để tính thể tích.
2. Topo cho General: dùng cấu trúc **radial-edge** (một cạnh nhiều mặt), half-edge là trường hợp đặc biệt. *Đây là chỗ đắt nhất nếu làm muộn.*
3. **Surface trait mở rộng ngay** cho Plane, Cylinder, Cone, Sphere, Torus, NURBS, và Curve cho Line, Arc, Ellipse, NURBS. Thuật toán có thể tới dần, nhưng *kiểu dữ liệu, file format, IFC (`IfcAdvancedBrep`) và Revit (`BRepBuilder` nhận NURBS)* phải sẵn từ bây giờ.
4. **Chiến lược kernel** (sửa D-30):
   - Giữ kernel Rust của Ferro cho phẳng, push-pull, imprint.
   - Đánh giá ngay **truck / monstertruck** (Rust, NURBS, boolean, chạy được WASM) cho mặt cong và boolean.
   - OCCT chỉ dùng trong plugin native (không build ra `wasm32-unknown-unknown`).
   - Mô hình dung sai: dung sai theo cạnh (tolerant edge) như Parasolid, để nhập file 3DM, SKP, IFC bẩn không vỡ.
5. **Fillet và chamfer** là lệnh hạng nhất (M6). Với tường tham số thì bo góc là tham số join, không phải phép B-rep.

---

## 6. F5: Quan hệ là dữ liệu

IFC có cả họ `IfcRel*`; Revit có join, cut, attach, host, room bounding. Ferro mới có host và L/T-join suy ra.

Đề xuất một **bảng quan hệ có kiểu**, lưu trong Document, có `Uid`, tham gia regenerate:

| Quan hệ | Ví dụ | Revit / IFC |
|---|---|---|
| `Join(a, b, kiểu)` | tường gặp tường, cột trong tường | Join Geometry / `IfcRelConnectsPathElements` |
| `Cut(void, target)` | **void cắt tường hoặc sàn, không phá hủy** | Cut Geometry, Opening / `IfcRelVoidsElement` |
| `Host(child, host)` | cửa trong tường | Host / `IfcRelFillsElement` |
| `Attach(a, b, mặt)` | đỉnh tường bám mái | Attach Top / Base |
| `Bounds(element, room)` | tường bao phòng | Room Bounding / `IfcRelSpaceBoundary` |
| `Connect(a, b, cổng)` | ống nối ống, dầm gối cột | Connector / `IfcRelConnectsPorts` |
| `Aggregate(whole, part)` | cầu thang gồm bậc và lan can | `IfcRelAggregates` |

Trả lời câu "dùng void hay theo SketchUp": **cả hai**.

- *Trong* Form: sửa trực tiếp kiểu SketchUp, nhanh, không có lịch sử (giữ D-38).
- *Giữa* các element: cắt bằng quan hệ `Cut`, không phá hủy. Void là một element (thường là Form) có vai trò void. Đổi tường hay đổi void thì phần cắt tự tính lại. Sang Revit thì map được thành Opening hoặc cut.

---

## 7. F6: Scene tách khỏi Document

Unreal tách *asset / editor world* khỏi *runtime*; USD tách *layer* khỏi *stage đã ghép*; Omniverse và các game engine đều dùng dữ liệu dẫn xuất (ECS) cho render và vật lý.

Đề xuất crate `ferro-scene`:

1. `Document` (sự thật, bất biến) → **Scene** (dẫn xuất, cập nhật tăng dần theo element thay đổi, D-24 đã cho biết element nào đổi).
2. Scene là ECS: mesh (có LOD và meshlet), **instance** (component D-71 → GPU instancing), material đã biên dịch, đèn, camera, collider, node point cloud, splat.
3. Render, picking, vật lý, phân tích, xuất USD và glTF **đều đọc Scene**, không đọc Document. AI vẫn đọc Document.
4. Streaming theo vùng (giống World Partition) cho mô hình liên kết lớn (D-37) và point cloud.

Đập: `SceneCache` trong `ferro-app` (đang build lại toàn bộ danh sách vertex mỗi khi có thay đổi).

---

## 8. F7–F12: quyết ngay, làm dần

### F7. Dữ liệu thực tế (point cloud, splat, mesh chụp ảnh)

Đây là mảng *ăn tiền trực tiếp* với thị trường scan-to-BIM.

1. Point cloud, splat và mesh lớn là **RealityAsset được liên kết, không nhúng**: tệp ngoài, có hash, có georeference. Lưu trong SQLite (D-21) là không khả thi với hàng tỷ điểm.
2. Nhập E57, LAS / LAZ, PTS, PLY; chuyển sang **COPC** (octree, LAZ, streaming được) làm định dạng làm việc. RCP / RCS của ReCap chỉ đọc qua Revit add-in.
3. Gaussian splat theo glTF `KHR_gaussian_splatting` (đang là release candidate, theo dõi bản chính thức).
4. Hiển thị ngoài lõi (out-of-core) theo octree, cắt bằng section và section box, **bắt điểm và mặt phẳng suy ra từ point cloud** (ferro-snap).
5. Scan-to-BIM: chọn vùng → nhận dạng mặt phẳng, trụ (RANSAC hoặc AI) → tạo element `Direct` hoặc `Parametric` có category, ghi lại nguồn `Reality(asset, region)` và độ lệch so với scan.

### F8. Phòng, vùng và mô hình năng lượng

1. `Room` là khối kín thật, tính từ element bao (`Bounds`), có space boundary cấp 2 (mặt chung giữa hai phòng, mặt ra ngoài).
2. Xuất mô hình năng lượng từ đó: **Honeybee JSON hoặc OpenStudio** (EnergyPlus), **gbXML**, IFC space boundary. Cửa ra Aperture, ô văng ra Shade, construction lấy từ F3.
3. `Site` có vĩ độ, kinh độ, cao độ, múi giờ, tệp thời tiết EPW, cùng hướng Bắc thật đã có (D-04).
4. Ánh sáng tự nhiên chạy qua Radiance hoặc tương đương, dưới dạng plugin.

### F9. Thời gian, mô phỏng và kết quả phân tích

1. Một **trục thời gian** chung cho mặt trời, hoạt cảnh, 4D thi công và mô phỏng vật lý.
2. Vật lý (rơi, va chạm, đi bộ thử, mô phỏng thoát hiểm) chạy trên collider dẫn xuất trong Scene. Ứng viên là **Rapier** (Rust thuần, chạy WASM), Jolt nếu cần hiệu năng native.
3. Kết quả phân tích (năng lượng, ánh sáng, bóng đổ, gió, kết cấu) là element **`AnalysisResult`**: lưới hoặc mesh có giá trị, gắn hash của dữ liệu đầu vào để biết khi nào lỗi thời. Hiển thị qua RenderStyle (D-49), giống Analysis Visualization Framework của Revit.

### F10. Lớp ý kiến (layering) cho option, liên kết và cộng tác

USD và IFC5 đều đi theo hướng *đối tượng được ghép từ nhiều lớp ý kiến* (layer, override, variant).

- Phương án thiết kế (D-32) chính là **variant**.
- Mô hình liên kết (D-37) là **reference**.
- Sửa cục bộ đè lên mô hình liên kết là **override**.
- Thay đổi của người khác trong phiên cộng tác là một lớp.

Đề xuất: mọi thuộc tính của element có **đường dẫn ổn định** (`element/uid/param/key`). Thay đổi là "ý kiến" trên đường dẫn. Đây là nền chung cho diff, merge (D-55), cộng tác thời gian thực (D-34, ứng viên Loro CRDT) và xuất USD / IFC5. Chưa cần cài composition đầy đủ, nhưng *định dạng đường dẫn và thay đổi phải chốt ngay*.

### F11. Lệnh và bộ phím

1. **Registry lệnh là dữ liệu**: tên ổn định, nhãn đã dịch, biểu tượng, ngữ cảnh. Phím tắt là một keymap người dùng sửa được.
2. Ba bộ có sẵn:
   - **SketchUp**: một phím cho dựng hình (L, R, C, A, P, F, M, Q, S, E, B, T).
   - **Revit**: mã hai chữ gõ liên tiếp cho đối tượng BIM (WA, DR, WN, FL, CM, RP, DI, AL, TR...).
   - **Ferro** (mặc định): dựng hình theo SketchUp, đối tượng BIM theo mã Revit; chữ đầu của mã Revit phải không trùng phím SketchUp đang dùng, nếu trùng thì mã Revit gõ qua ô lệnh.
3. Có **ô lệnh / command palette** (gõ tên hoặc mã).

### F12. Đa ngôn ngữ, báo lỗi, kiểm tra cho agent

Đã duyệt trong ADR 0003. Nhắc lại phần móng: chuỗi giao diện đi qua `tr!`, chuỗi ổn định (tên lệnh, mã lỗi, khóa tham số) không bao giờ dịch, và `ferro-check` dùng chung cho người, agent và CI.

---

## 9. Giữ nguyên

Những quyết định sau đã đối chiếu và **nên giữ**:

- D-01–D-10: đơn vị, tọa độ, dung sai, `Point3` / `Vec3`, plane có hệ trục.
- D-13: hai loại id.
- D-17–D-19: Command, Transaction, regenerate, lõi thuần.
- D-24, D-25: tài liệu bất biến, id topo.
- D-26: AI đề xuất.
- D-33: view là element.
- D-35: WASM.
- D-44–D-47: AI.
- D-48–D-50: renderer wgpu, style, AI render.
- D-51, D-61: plugin WASM có quyền.
- D-56–D-58: IFC, Revit link, DWG.
- D-62: ngân sách hiệu năng.

---

## 10. Việc cần chủ dự án quyết

1. Duyệt F1–F6 là phạm vi **Milestone 5 "Móng 2"** và tạm dừng phần còn lại của M4 cho tới khi xong F1, F2, F3, F5?
2. F1: mặc định xuất hình tự do sang Revit là **DirectShape** (nhanh, không sửa được trong Revit) hay **loadable family** (nặng hơn, có type)?
3. F4: cho phép đánh giá truck / monstertruck làm kernel mặt cong (thêm phụ thuộc vào `ferro-core`, cần ADR)?
4. F11: bộ phím mặc định là **Ferro (lai)**?

## Nguồn

- Revit API không tạo được in-place family: [Revit API Forum, "Creating in-place families"](https://forums.autodesk.com/t5/revit-api-forum/creating-in-place-families/m-p/12141347)
- DirectShape và category hợp lệ: [DirectShape.IsValidCategoryId](https://help.autodesk.com/cloudhelp/2026/ENU/Revit-API-MainReference/files/html/c8881d31-d410-842a-1375-5ba688191cf8.htm), [BRepBuilderSurfaceGeometry](https://help.autodesk.com/cloudhelp/2026/ENU/Revit-API-MainReference/files/html/c21d4101-cc15-1990-ee9f-994b487bf95d.htm)
- Dữ liệu dạng hạt của Autodesk: [AEC Magazine, "Autodesk's granular data strategy"](https://aecmag.com/technology/autodesks-granular-data-strategy/), [AEC Data Model API](https://aps.autodesk.com/developer/overview/aec-data-model-api)
- IFC5: [buildingSMART IFC5-development](https://raw.githubusercontent.com/buildingSMART/IFC5-development/main/README.md), [IFC5 introduction](https://lvanlier.github.io/ifc5/introduction)
- OpenUSD Core Spec 1.0: [Linux Foundation](https://www.linuxfoundation.org/press/alliance-for-openusd-announces-core-specification-1.0-the-universal-language-for-building-3d-worlds)
- OpenPBR: [Adobe, OpenPBR Surface v1.0](https://blog.adobe.com/en/publish/2024/06/04/openpbr-surface-v-10-major-step-forward-for-digital-materials)
- Gaussian splat trong glTF: [Digital Production, 02/2026](https://digitalproduction.com/2026/02/04/gltf-learns-to-splat-without-losing-the-plot/)
- Honeybee: [honeybee-schema](https://github.com/ladybug-tools/honeybee-schema/wiki/1.1-Model-Schema)
- COPC: [LiDAR News](https://lidarnews.com/cloud-optimized-point-clouds-copc/)
- Kernel Rust: [truck](https://github.com/ricosjp/truck), [monstertruck](https://docs.rs/monstertruck)
- Speckle: [object schema](https://docs.speckle.systems/developers/data-schema/object-schema)
- CRDT: [Loro 1.0](https://loro.dev/blog/v1.0)
