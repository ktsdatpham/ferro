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
