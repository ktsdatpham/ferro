# C. Hình học · D. Parametric

## C. Hình học

### Q-C1. Geometry kernel: tự viết hay dùng có sẵn? 🔴

**Vì sao quan trọng:** Kernel quyết định Ferro dựng được hình gì (mặt cong? bo
góc? boolean phức tạp?), độ bền (boolean có "nổ" không), license và tốc độ.

| Phương án | Được | Mất | Ai dùng |
|---|---|---|---|
| **A. Tự viết bằng Rust** (half-edge, mặt phẳng trước, NURBS sau) | Kiểm soát hoàn toàn, thuần Rust, chạy được trên web/iPad, license sạch | Boolean và NURBS bền là việc nhiều năm | SketchUp (kernel riêng, mặt phẳng), Plasticity dùng Parasolid |
| **B. OpenCascade (OCCT)** qua binding | Kernel B-rep đầy đủ, miễn phí, đã dùng 30 năm | C++, LGPL, nặng, API khó, boolean cũng có lỗi, khó build cho web/iOS | FreeCAD, Bonsai (một phần), nhiều app mở |
| **C. Parasolid / ACIS** (thương mại) | Tốt nhất thế giới | Rất đắt, đóng, mâu thuẫn mã nguồn mở | Shapr3D, Plasticity, SolidWorks, BricsCAD |
| **D. Lai:** tự viết lõi mặt phẳng (90% công việc kiến trúc là mặt phẳng) + **Manifold** cho boolean mesh bền + **truck** hoặc **OCCT tùy chọn** cho NURBS về sau | Nhanh có sản phẩm, bền, mở rộng được | Hai biểu diễn (B-rep và mesh) phải chuyển qua lại cẩn thận | — |

**Khuyến nghị:** **D.** Kiến trúc chủ yếu là mặt phẳng; SketchUp thống trị chỉ
với mặt phẳng. Tự viết half-edge cho mặt phẳng (kiểm soát trải nghiệm push-pull),
boolean dựa trên thuật toán có predicate chính xác (Q-C3) hoặc thư viện Manifold.
NURBS là giai đoạn sau, cắm qua trait `Surface`.
**Độ khó đổi:** 🔴
**TaD trả lời:** ☐ A ☐ B ☐ C ☐ D · ghi chú:

---

### Q-C2. Biểu diễn hình học gốc 🔴

| Phương án | Được | Mất |
|---|---|---|
| **A. Chỉ mesh tam giác** | Đơn giản, render thẳng | Không có "mặt" thật, push-pull khó, không chính xác |
| **B. B-rep mặt phẳng có lỗ** (kiểu SketchUp làm đúng) | Đủ cho 90% kiến trúc, push-pull tự nhiên | Mặt cong phải xấp xỉ |
| **C. B-rep đầy đủ NURBS** | Chính xác mọi hình | Rất khó |
| **D. B lúc đầu, kiến trúc cho C** (mặt có trait `Surface`; `Plane` là triển khai đầu tiên) + **SubD** cho hình tự do về sau | Ra nhanh, không bị kẹt | Phải giữ kỷ luật trừu tượng |

**Khuyến nghị:** **D.** Mặt cong kiến trúc phổ biến (tường cong, vòm, mái cong)
làm qua `Arc` extrude và lofted surface ở giai đoạn 2.
**Độ khó đổi:** 🔴
**TaD trả lời:** ☐ A ☐ B ☐ C ☐ D · ghi chú:

---

### Q-C3. Độ bền số học (robustness) 🔴

**Vì sao quan trọng:** Lỗi "mặt bị thủng", "boolean thất bại", "đường không khép"
đến từ sai số dấu phẩy động. Đây là nguồn gốc 50% bug của mọi app CAD.

| Phương án | Được | Mất |
|---|---|---|
| **A. f64 + dung sai** (hiện tại) | Nhanh, đơn giản | Trường hợp biên vẫn sai |
| **B. Predicate chính xác** (orient2d/orient3d kiểu Shewchuk, crate `robust`) cho mọi quyết định "trái/phải/trên/dưới", f64 cho tọa độ | Quyết định topo luôn nhất quán, chỉ chậm hơn ở ca khó | Phải dùng đúng chỗ |
| **C. Số học chính xác toàn bộ** (rational/exact) | Không bao giờ sai | Chậm 10–100 lần |

**Khuyến nghị:** **B.** Quy tắc: mọi quyết định topo (giao nhau? bên trong?) qua
predicate chính xác; tọa độ lưu f64; dung sai chỉ dùng để gộp đỉnh (D-06).
**Độ khó đổi:** 🟠
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-C4. Tham chiếu bền khi hình thay đổi (topological naming) 🔴🔴

**Vì sao quan trọng (vấn đề lớn nhất):** Kích thước khóa, cửa sổ đặt trên mặt
tường, vật liệu gán cho một mặt, AI nói "mặt phía nam"... tất cả trỏ vào một mặt
hoặc cạnh. Push-pull, boolean làm topo đổi → mặt bị tạo lại → **tham chiếu gãy**.
FreeCAD khổ vì cái này hơn 10 năm. Revit giải quyết bằng `Reference` ổn định
gắn với tham số sinh hình, không gắn với chỉ số mặt.

| Phương án | Được | Mất | Ai dùng |
|---|---|---|---|
| **A. Tham chiếu theo chỉ số** (face #3) | Đơn giản | Gãy khi topo đổi | FreeCAD cũ |
| **B. Đặt tên theo lịch sử sinh hình**: mỗi mặt có tên ổn định suy từ thao tác sinh ra nó (vd `extrude(profile#e2).side`) | Bền qua chỉnh sửa tham số | Phức tạp, phải thiết kế cho mọi thao tác | Parasolid/Onshape, Revit (dạng tương tự), FreeCAD 1.0 |
| **C. ID bền gắn vào mặt/cạnh, được thao tác chuyển giao** (push-pull giữ ID mặt bị kéo, mặt mới sinh ID mới, split thì một bên giữ ID gốc + ghi quan hệ "sinh ra từ") | Tự nhiên với modelling trực tiếp kiểu SketchUp; dễ hiểu | Mỗi thuật toán phải ghi nhận nguồn gốc (lineage) | Một số kernel direct modelling |
| **D. Tham chiếu ngữ nghĩa** cho element BIM (mặt "ngoài" của tường, cạnh "đỉnh" của sàn) + C cho Form | Element BIM gần như không bao giờ gãy; AI nói bằng ngữ nghĩa | Phải định nghĩa tên ngữ nghĩa cho mỗi category | Revit (`HostObjectUtils.GetSideFaces`) |

**Khuyến nghị:** **C + D.** Mọi thuật toán topo phải trả về **bảng nguồn gốc**
(mặt/cạnh nào sinh từ mặt/cạnh nào). Element BIM tham chiếu theo tên ngữ nghĩa
(`WallFace::Exterior`). Đây là phần phải viết spec riêng và test nặng nhất.
**Độ khó đổi:** 🔴🔴 (không thể thêm sau)
**TaD trả lời:** ☐ A ☐ B ☐ C ☐ D ☐ C+D · ghi chú:

---

### Q-C5. Hình học 2D (offset, boolean đa giác, tô hatch) 🟢

| Phương án | Được | Mất |
|---|---|---|
| **A. Tự viết** | Kiểm soát | Lâu, nhiều bug |
| **B. Dùng thư viện Rust** (`i_overlay` / `geo` cho boolean và offset đa giác, `lyon` cho tessellation 2D, `earcut`/`spade` cho tam giác hóa) | Đã được kiểm nghiệm, nhanh | Phụ thuộc thư viện ngoài |

**Khuyến nghị:** **B** (đúng nguyên tắc "mượn đúng chỗ"), bọc sau API của Ferro để
thay được.
**TaD trả lời:** ☐ A ☐ B · ghi chú:

---

### Q-C6. Modelling trực tiếp hay theo lịch sử? 🔴

| Phương án | Được | Mất | Ai dùng |
|---|---|---|---|
| **A. Trực tiếp** (kéo mặt nào sửa mặt đó, không có cây lịch sử) | Nhanh, trực quan, AI dễ thao tác | Khó sửa "ý đồ" ban đầu | SketchUp, Shapr3D (chủ yếu) |
| **B. Theo lịch sử** (cây feature, sửa bước đầu thì tính lại hết) | Tham số hóa mạnh | Chậm, tham chiếu gãy, người không chuyên khó dùng | SolidWorks, Fusion, FreeCAD |
| **C. Element BIM là tham số** (tường = đường + độ dày + chiều cao), **Form là trực tiếp**, không có cây lịch sử toàn cục | Đúng như Revit/ArchiCAD: phần lớn tham số nằm ở element, không ở lịch sử | Form không có tham số hóa lịch sử |

**Khuyến nghị:** **C.** Tham số hóa sâu hơn đến từ node graph (Q-D2) chứ không phải
cây lịch sử kiểu cơ khí.
**Độ khó đổi:** 🔴
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-C7. Hình học hiển thị (tessellation) và LOD 🟠

**Vì sao quan trọng:** Render cần tam giác. Đồ án lớn có hàng triệu element.

**Khuyến nghị (không cần chọn, cần đồng ý nguyên tắc):**
1. Tessellation là **dữ liệu dẫn xuất** (cache), không lưu là sự thật.
2. Mỗi element có tối đa 3 mức chi tiết: `Coarse` (hộp/khối), `Medium`, `Fine`
   (giống Detail Level của Revit, dùng luôn cho render ở xa).
3. Element giống nhau (cửa cùng type, cây cùng loại) dùng **instancing**.
4. Tessellation chạy song song trên snapshot (Q-B6), không chặn UI.

**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-C8. Point cloud và mesh scan 🟠

**Vì sao quan trọng:** Cải tạo nhà cũ (rất phổ biến ở VN) bắt đầu từ scan; TaD đã
có hướng scan LiDAR/RoomPlan. AI có thể chuyển scan thành element.

| Phương án | Được | Mất |
|---|---|---|
| **A. Point cloud là element tham chiếu** (`PointCloudLink`, không sửa được, render octree, snap được) | Đúng chuẩn Revit/ArchiCAD | Phải làm renderer octree |
| **B. Làm sau qua plugin** | Tập trung lõi | Mất một lợi thế cạnh tranh rõ ràng |

**Khuyến nghị:** **A, ở milestone 3–4**, kiến trúc renderer chuẩn bị từ đầu cho
dữ liệu điểm lớn (streaming). Scan → element là một tính năng AI hàng đầu.
**TaD trả lời:** ☐ A ☐ B · ghi chú:

## D. Parametric và ràng buộc

### Q-D1. Hệ ràng buộc (constraint) 🔴

**Vì sao quan trọng:** Kiến trúc sư cần "khóa khoảng cách này 1200", "hai cửa sổ
cách đều", "tường này luôn bám trục B". Revit có lock/EQ; SketchUp không có.

| Phương án | Được | Mất | Ai dùng |
|---|---|---|---|
| **A. Không có ràng buộc** | Đơn giản | Sửa một chỗ phải sửa tay nhiều chỗ | SketchUp |
| **B. Ràng buộc quan hệ đơn giản qua regeneration** (bám level, bám grid, host, căn lề, cách đều) — giải theo thứ tự đồ thị phụ thuộc, không cần solver | Bao 80% nhu cầu, nhanh, dễ hiểu | Không giải được hệ ràng buộc vòng |
| **C. Solver ràng buộc hình học đầy đủ** (kiểu sketcher của CAD cơ khí, vd SolveSpace/planegcs) | Mạnh nhất | Chậm, khó hiểu với kiến trúc sư, hệ quá ràng buộc gây lỗi khó chịu | Revit sketch, FreeCAD, Fusion |
| **D. B cho model + C chỉ trong sketch profile và family editor** | Đúng như Revit | Hai cơ chế | Revit |

**Khuyến nghị:** **D**, làm B trước (milestone 2), solver sketch sau.
**Độ khó đổi:** 🟠
**TaD trả lời:** ☐ A ☐ B ☐ C ☐ D · ghi chú:

---

### Q-D2. Node graph kiểu Grasshopper/Dynamo 🟠

| Phương án | Được | Mất | Ai dùng |
|---|---|---|---|
| **A. Node graph trong lõi** (graph là element trong Document, output là element thật, chạy trong regeneration) | Parametric mạnh, đồng bộ với BIM | Lõi nặng hơn | Grasshopper (ngoài Rhino), Dynamo (ngoài Revit) |
| **B. Node graph là plugin** | Lõi gọn | Kém tích hợp |
| **C. Không node, AI viết script thay** | AI thay vai trò node | Người dùng không chỉnh trực quan được |

**Khuyến nghị:** **A, nhưng muộn (milestone 4+)**, và mỗi node = một command/algorithm
có sẵn. AI tạo được graph (giải thích được, chỉnh được) tốt hơn script hộp đen.
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-D3. Biểu thức tham số (formula) 🟠

Revit cho phép `Width = Height / 2`. Cần một ngôn ngữ biểu thức.

| Phương án | Được | Mất |
|---|---|---|
| **A. Ngôn ngữ biểu thức tự viết nhỏ** (số học, đơn vị, if, hàm toán, tham chiếu parameter) | Tất định, an toàn, có đơn vị | Phải viết parser |
| **B. Nhúng Rhai/Lua** | Mạnh | Quá mạnh cho công thức, khó đảm bảo tất định |

**Khuyến nghị:** **A**, có **kiểm tra đơn vị** (`mm + m²` báo lỗi), tương thích
cú pháp gần với công thức Revit để dễ chuyển đổi.
**TaD trả lời:** ☐ A ☐ B · ghi chú:

---

### Q-D4. Ngôn ngữ script cho người dùng/plugin 🟠

| Phương án | Được | Mất | Ai dùng |
|---|---|---|---|
| **A. Python** | Kiến trúc sư/BIM biết Python nhất (Dynamo, pyRevit), AI viết rất tốt | Nhúng Python khó trên iPad/web, chậm, khó sandbox | Blender, Dynamo, pyRevit |
| **B. JavaScript/TypeScript** | Chạy được mọi nơi, sandbox tốt | Ít phổ biến với dân AEC | Figma, Speckle |
| **C. WASM (viết bằng Rust/C#/bất kỳ, chạy sandbox)** | Nhanh, an toàn, đa ngôn ngữ | Rào cản cao cho người mới | Zed, Figma (lõi plugin) |
| **D. C cho plugin + A cho script nhanh trên desktop** | Có cả an toàn lẫn tiện | Hai đường |

**Khuyến nghị:** **D.** Plugin phân phối = WASM (Q-K1); script cá nhân = Python trên
desktop (gọi cùng command API). Cả hai chỉ thay đổi model qua command.
**TaD trả lời:** ☐ A ☐ B ☐ C ☐ D · ghi chú:
