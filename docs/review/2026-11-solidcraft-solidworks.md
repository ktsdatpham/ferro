# Tham khảo SolidCraft và SolidWorks

- **Ngày:** 2026-11-23
- **Trạng thái:** Ghi chép tham khảo. Các đề xuất ở mục 4 cần chủ dự án duyệt; phần kernel chờ báo cáo đánh giá M5-9.2.

## 1. SolidCraft là gì

[SolidCraft](https://github.com/tangency-labs/solidcraft) là phần mềm CAD tham số mã nguồn mở viết bằng Rust, kiểu Fusion 360: sketch có ràng buộc, cây lịch sử feature (extrude, revolve, fillet, shell, sweep, loft…), assembly, sheet metal, render. Chạy được trên desktop, trên trình duyệt (WASM), dưới dạng CLI không giao diện, và làm **MCP server cho AI agent** (`solidcraft-cli mcp`, có chế độ `--connect` để điều khiển app đang mở). Đây đúng là mô hình Ferro đang đi.

- **Giấy phép:** mã nguồn MIT hoặc Apache-2.0, ghép được với MPL-2.0 của Ferro.
  - **Ngoại lệ phải tránh:** giao diện và thương hiệu (`crates/ui-kit`, `crates/look`, `crates/theme`, logo, icon) nằm dưới *SolidCraft Brand and Interface Licence*. Muốn dùng trong phần mềm khác phải xin phép bằng văn bản. Ferro **không chép** các crate đó.
- **Độ chín:** tự nhận là "early but usable for simple parts". 75 trên 76 chi tiết mẫu khớp thể tích, diện tích và trọng tâm.

## 2. Phát hiện quan trọng nhất: kernel solidcore

[solidcore](https://github.com/tangency-labs/solidcore) là kernel B-rep viết bằng Rust an toàn, port từ **SMLib** trong USD BRep của NVIDIA (Apache-2.0), rồi phát triển tiếp.

- **Hình học chính xác:** mặt giải tích và NURBS, giao cắt mặt–mặt, boolean, fillet và chamfer (rolling-ball blend), offset và shell, sweep, skin, vá hở (healing), tính khối lượng, STEP, tessellation.
- **Mục tiêu thiết kế:** không `unsafe`, không panic với dữ liệu xấu, kết quả tất định, không trạng thái toàn cục, build được `wasm32-unknown-unknown`.

Những mục tiêu này trùng gần như từng chữ với nguyên tắc của `ferro-core` (D-19, D-35, D-53).

**SolidCraft từng dùng truck, rồi chuyển sang SMLib/solidcore, và đã bỏ hẳn truck** (`docs/kernel-smlib-history.md`). Đây là bằng chứng thực tế nặng ký cho câu hỏi kernel mặt cong của Ferro. Agent đánh giá kernel (M5-9.2) đã được giao thêm solidcore và phải tóm tắt lý do họ bỏ truck.

**Lưu ý:** API của solidcore còn ở 0.0.x (thử nghiệm), repo nặng (khoảng 560 MB kèm dữ liệu kiểm thử), và đi kèm công cụ build riêng. Nếu chọn thì phải bọc sau một **ranh giới duy nhất**, như SolidCraft đang làm (chỉ đi qua `solidcore-ops`, `solidcore-tess`, `solidcore-record`).

## 3. Bài học kiến trúc từ SolidCraft (đối chiếu Ferro)

| SolidCraft làm | Ferro hiện có | Nên làm |
|---|---|---|
| Phân lớp crate L0–L5 kiểm bằng `cargo xtask layers`; crate "shared" tự đứng một mình | Lớp trong `ferro-core` kiểm bằng `scripts/lint.py` | Mở rộng lint sang cấp crate (app, scene, io…) |
| Một **seam** duy nhất sang kernel (3 crate công khai, danh sách đường vào chỉ được giảm) | Kernel tự viết nằm trong `ferro-core` | Tạo crate seam `ferro-kernel` trước khi đưa kernel mặt cong vào |
| "Everything is a command": UI, CLI, MCP, trình duyệt chạy cùng lệnh | Command + MCP 1:1 (D-17, D-44) | Giữ; thêm **CLI không giao diện** (`ferro-cli`) để agent và CI chạy kịch bản |
| B-rep dạng bản ghi (schema 4, tên theo STEP, pcurve, dung sai theo cạnh và đỉnh, **thứ tự chuẩn để đặt tên bền vững**) | Half-edge + id topo bền vững + lineage (D-25) | Khi sang mặt cong: dùng bản ghi kiểu STEP và thứ tự chuẩn như họ |
| **Naming corpus**: dựng lại 154 thiết kế, kiểm từng tên mặt và cạnh | Test riêng lẻ | Thêm "corpus" mô hình mẫu, kiểm id và lineage sau mọi thay đổi kernel |
| Crash report và Report a bug: người dùng xem trước, mở GitHub, không gửi ngầm | Vừa làm M5-12.5, cùng nguyên tắc | Thêm ảnh chụp màn hình tùy chọn như họ |
| Đa ngôn ngữ: catalog JSON nhúng, ID ổn định, câu trọn vẹn có placeholder đặt tên, 30 ngôn ngữ | Chưa làm (M5-12.4, ADR 0003 chọn Fluent) | Giữ Fluent hoặc theo JSON; quan trọng là ID ổn định và dịch nguyên câu |
| Point cloud: PLY, PCD, E57, XYZ/PTS; chỉ làm tham chiếu, không thành khối; nạp lại từ file | Kế hoạch F7 | Làm theo, cộng COPC để xem tập điểm lớn |
| Plugin WASM (WIT component), chỉ chạy khi người dùng bấm | D-51 | Tham khảo WIT của họ khi làm plugin |
| Render path tracer GPU và CPU, vật liệu, HDR | D-48 | Tham khảo khi làm Render workspace |

## 4. SolidWorks

SolidWorks là phần mềm thương mại đóng (Dassault), không có mã nguồn trên GitHub. Trên GitHub chỉ có ví dụ dùng **SolidWorks API** (COM) và macro. Những gì đáng học là *khái niệm* công khai:

- **FeatureManager tree và rebuild:** cây lịch sử. Đổi kích thước thì dựng lại từ feature bị đổi.
- **Configurations và Design Tables:** một chi tiết có nhiều biến thể lấy từ bảng tham số. Gần với FamilyType của Revit và Design Options (D-32) của Ferro.
- **Sketch relations và trạng thái fully defined:** sketch báo rõ còn thiếu bao nhiêu ràng buộc.
- **Mates** trong assembly.
- **Cấu trúc API dạng cây đối tượng** (`ISldWorks → IModelDoc2 → IFeatureManager → IFeature`), tương tự Revit API. Bản đồ API của Ferro (`docs/api/TREE.md`) và Python SDK đi đúng hướng này.

## 5. Đề xuất đưa vào cây việc

1. **M5-9.2** (đang chạy): đánh giá cả solidcore, đặt ngang truck và monstertruck.
2. **M5-16 (mới):** crate seam `ferro-kernel`. Mọi phép hình học của Form đi qua đây, để thay kernel mà không đụng phần trên.
3. **M5-17 (mới):** corpus mô hình mẫu cùng bộ kiểm id và lineage topo (học "naming corpus").
4. **M5-18 (mới):** `ferro-cli` chạy lệnh và kịch bản không cần giao diện, dùng cho agent, CI và benchmark.
5. **Sau này:** cân nhắc lấy bộ giải ràng buộc sketch của SolidCraft (crate `sketch`, MIT/Apache) cho D-54 (ràng buộc trong sketch và family editor), sau khi đánh giá riêng.

## Nguồn

- [SolidCraft (GitHub)](https://github.com/tangency-labs/solidcraft): README, `docs/architecture.md`, `docs/layers.md`, `docs/kernel-smlib-history.md`, `docs/native-brep.md`, `docs/crash-reports.md`, `docs/localization.md`, `docs/point-clouds.md`, `LICENSE-BRAND`
- [solidcore (GitHub)](https://github.com/tangency-labs/solidcore): README
- [Hackaday: SolidCraft](https://hackaday.com/2026/10/17/SolidCraft-is-parametric-automation-friendly-cad-for-desktop-and-browser/)
- [The Menon Lab: SolidCraft và MCP agents](https://themenonlab.blog/blog/solidcraft-open-source-fusion-360-rust-mcp-agents)

## 6. Kết luận chi tiết (bổ sung sau khi đọc sâu)

### 6.1 Từ SolidCraft: 7 bài học kỹ thuật

1. **Kernel.** truck không đủ cho CAD thật; họ đã chuyển hẳn sang SMLib/solidcore. Ferro nên chờ báo cáo M5-9.2, nhưng solidcore là ứng viên số 1.
2. **Tên bền vững đi theo lịch sử của kernel, không đoán bằng hình học** (`docs/book/src/dev/naming.md`):
   - Mỗi phép toán báo mặt và cạnh mới sinh ra từ đâu.
   - Mặt được đặt tên theo vai trò (`F2:side:l4`, `top`, `blend(edge)`).
   - Mảnh của một mặt bị cắt được phân biệt bằng các mặt bao quanh nó.
   - Cạnh được gọi bằng tên hai mặt nó chia.
   - Tên không chắc chắn bị đánh dấu `unverified`.
   - **Không bao giờ tự gắn lại sang mặt khác mà không báo** ("the worst thing a CAD tool can do").

   Ferro đã có id topo cộng lineage (D-25) đúng hướng này. Cần thêm hai thứ: cờ `exact / unverified`, và luật "tham chiếu mất thì báo lỗi, không đoán".
3. **Kiểm thử theo "oracle"** (`docs/oracle.md`):
   - Mỗi ca kiểm có đáp án tính tay (thể tích, diện tích, trọng tâm) hoặc lấy từ một kernel độc lập (FreeCAD chạy như chương trình ngoài).
   - Không ca nào lấy đáp án từ phần mềm thương mại.
4. **Fuzz độ bền** (`docs/robustness.md`):
   - Sinh ngẫu nhiên các thiết kế "giống thật": đặt theo lưới 2,5 mm để mặt trùng nhau và tiếp xúc xảy ra thường xuyên.
   - Kiểm sau mỗi bước: khối kín, thể tích đi đúng chiều.
   - Kiểm hằng đẳng thức boolean: V(A∪B) + V(A∩B) = V(A) + V(B).
   - Mỗi seed chạy trong tiến trình riêng có giới hạn thời gian; ca lỗi được rút gọn tự động.
5. **File dạng container:** `document.json` có phiên bản, cộng các blob đặt tên theo SHA-256 (mesh, STEP, ảnh) và một **model cache** của B-rep. Nhờ vậy mở file không phải dựng lại từ đầu.
6. **Configurations:** bảng biến thể (tham số, ẩn/hiện feature, vật liệu, vẻ ngoài). Có cờ `matches` báo khi thiết kế đã lệch khỏi dòng đang chọn.
7. **Mọi thứ là lệnh, và tài liệu sinh từ registry lệnh.** MCP chạy được hai kiểu: không giao diện, hoặc nối vào app đang mở. Có công cụ `view.pick` bắn tia để agent "nhìn" mô hình; render chạy dạng job bất đồng bộ.

### 6.2 Từ SolidWorks: 12 khái niệm đáng lấy (và cách Ferro làm)

| SolidWorks | Ý nghĩa | Ferro nên làm |
|---|---|---|
| **FeatureManager tree + rollback bar** | Cây lịch sử; kéo thanh rollback để chèn feature vào giữa | Không áp lịch sử cho cả mô hình (D-38, Revit cũng không). Dùng **Project Browser** (level → category → element, type, vật liệu, option) làm cây chính; lịch sử feature chỉ có trong **family editor / component** |
| **Parent/child + "What's Wrong?"** | Biết feature nào phụ thuộc feature nào; danh sách lỗi và cảnh báo của lần rebuild, chỉ thẳng chỗ hỏng | **Bảng Problems** liệt kê cảnh báo regenerate và tham chiếu hỏng, bấm vào thì chọn đối tượng, kèm gợi ý sửa; xem quan hệ của một đối tượng |
| **Configurations + Design Tables** | Một chi tiết nhiều biến thể; bảng Excel điều khiển kích thước và ẩn/hiện feature | FamilyType = configuration; **Type Catalog dạng bảng**, nhập/xuất CSV; AI điền bảng. Design Options (D-32) = configuration ở cấp dự án |
| **Equations / Global Variables** | Kích thước liên kết bằng công thức | **Công thức tham số có đơn vị** (dựa trên QuantitySpec vừa làm), bắt buộc cho family |
| **Sketch relations + Fully Defined** | Sketch báo đủ, thiếu hay thừa ràng buộc bằng màu | Bộ giải ràng buộc (D-54) báo số bậc tự do còn lại; cân nhắc lấy crate `sketch` của SolidCraft (MIT) |
| **Instant3D** | Kéo mặt hay kích thước trực tiếp trong 3D, có thước | Push-pull đã có; thêm **kích thước tạm** khi chọn đối tượng (Revit), gõ số để sửa |
| **Hole Wizard / Toolbox / Library Features** | Feature thông minh theo tiêu chuẩn (lỗ, bu lông) | **Trình hướng dẫn theo catalog**: cửa và cửa sổ theo TCVN, cầu thang theo quy chuẩn, dựa trên các construction-system pack (D-67) |
| **Weldments** | Thanh kết cấu quét theo đường, có thư viện tiết diện, cắt góc, **cut list** ghi chiều dài | **Structural member**: thép nhẹ, gỗ, lan can, khung vách kính quét profile theo đường, kèm bảng cắt. Rất hợp pack thép tiền chế và khung nhẹ |
| **Lightweight components / SpeedPak / Large Assembly Mode** | Chỉ nạp phần hiển thị, khi cần mới nạp đủ; bản rút gọn giữ lại mặt cần cho mate | Lưu **model cache hiển thị** trong `.fro`; mô hình liên kết (D-37) chỉ nạp mesh và các mặt cần để bắt điểm và host; nạp đủ khi bắt đầu sửa |
| **Document Manager API** | Đọc file (thuộc tính, BOM) mà không cần mở SolidWorks | `ferro-cli info` và đọc `.fro` bằng Python không cần app |
| **API cây đối tượng + xCAD** (MIT) | `ISldWorks → IModelDoc2 → FeatureManager`; xCAD sinh property page từ mô hình dữ liệu, "macro feature" là tham số vào, hình ra | Bản đồ API và Python SDK đã đi hướng này. Thêm: **bảng thuộc tính của công cụ sinh tự động từ schema lệnh**; plugin dạng "tham số vào, hình ra" cho element tham số |
| **PDM, eDrawings, Costing, Sustainability, Simulation** | Quản lý phiên bản và vòng đời, trình xem nhẹ có markup, ước tính chi phí và carbon tức thì, mô phỏng trên cùng mô hình | Nhánh và đề xuất (D-55) cộng trạng thái vòng đời; trình xem web + BCF; **dự toán và carbon tức thì** từ vật liệu (M5-4) × khối lượng (M5-2); phân tích là biểu diễn dẫn xuất (F9) |

### 6.3 Thay đổi vào cây việc

- M5-17 mở rộng: corpus mô hình **cộng oracle tính tay cộng fuzz hằng đẳng thức boolean**.
- M5-19 (mới): **bảng Problems** kiểu "What's Wrong"; tham chiếu có cờ exact/unverified, mất thì báo, không đoán.
- M5-20 (mới): **model cache** trong `.fro` (mesh hiển thị và B-rep) để mở nhanh và nạp nhẹ mô hình liên kết.
- M6 thêm: kích thước tạm, Type Catalog dạng bảng + CSV, công thức tham số, structural member + cut list, trình hướng dẫn theo catalog, dự toán và carbon tức thì, bảng thuộc tính sinh từ schema lệnh.

### Nguồn bổ sung

- [SolidWorks Help: FeatureManager Design Tree](https://help.solidworks.com/2020/English/SolidWorks/sldworks/c_featuremanager_design_tree_overview.htm), [Rollback bar](https://help.solidworks.com/2015/ENGLISH/SolidWorks/sldworks/c_rollback_bar.htm), [What's Wrong?](https://help.solidworks.com/2019/english/SolidWorks/sldworks/hidd_new_whats_wrong.htm)
- [SolidWorks Help: Lightweight Components](https://help.solidworks.com/2023/English/SolidWorks/Sldworks/c_Lightweight_Components_SWassy.htm), [SpeedPak](https://help.solidworks.com/2025/English/SolidWorks/sldworks/c_speedpak_oh.htm), [Large assembly tips](https://blogs.solidworks.com/tech/2017/04/large-assembly-tips-tricks.html)
- [xCAD.NET (MIT)](https://github.com/xarial/xcad), [CodeStack SOLIDWORKS API examples](https://github.com/xarial/codestack)
- SolidCraft: `docs/book/src/dev/naming.md`, `docs/oracle.md`, `docs/robustness.md`, `docs/design-container.md`, `docs/configurations.md`, `docs/mcp.md`
