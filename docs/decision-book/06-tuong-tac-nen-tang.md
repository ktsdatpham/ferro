# F. Tương tác và UI · L. Nền tảng

## F. Tương tác

### Q-F1. Bộ công cụ giao diện (UI toolkit) 🔴

**Vì sao quan trọng:** Quyết định tốc độ phát triển UI, cảm giác app, khả năng
chạy web/iPad, và việc người đóng góp (và AI) viết UI dễ hay khó.

| Phương án | Được | Mất | Ai dùng |
|---|---|---|---|
| **A. egui** (Rust, immediate mode) | Cực nhanh để làm, thuần Rust, chạy mọi nơi kể cả web | Trông như tool kỹ thuật, khó làm UI đẹp/phức tạp, trợ năng yếu | Rerun, nhiều tool nội bộ |
| **B. UI web (TypeScript/React) trong webview** + viewport wgpu native (kiểu Tauri) | Nhiều người làm được, AI viết React cực tốt, UI đẹp nhanh, **dùng lại y nguyên cho bản web** | Ghép webview với viewport native phức tạp (độ trễ, focus, IME), hai ngôn ngữ | VS Code, Figma desktop (Electron), Linear |
| **C. Native từng nền tảng** (SwiftUI Mac/iPad, WinUI Windows) qua `ferro-ffi` | Cảm giác native tốt nhất, iPad tuyệt vời | Làm UI 2–3 lần, ít người đóng góp được | Shapr3D (một phần), app Apple |
| **D. Toolkit Rust retained mode** (Slint, Iced, Xilem, hoặc tự viết kiểu GPUI của Zed) | Một codebase Rust, nhanh, đẹp được | Hệ sinh thái non, tự viết thì rất tốn | Zed (GPUI tự viết) |

**Khuyến nghị:** **A cho milestone 1–2** (chứng minh công cụ vẽ, không đầu tư UI),
**quyết định B hay D ở cuối milestone 2** khi đã biết rõ yêu cầu. Điều kiện bắt
buộc dù chọn gì: **UI chỉ nói chuyện với lõi qua command + query + event** (không
gọi hàm lõi trực tiếp), nên thay UI không ảnh hưởng lõi.
**Độ khó đổi:** 🟠 (nhờ ranh giới command)
**TaD trả lời:** ☐ A ☐ B ☐ C ☐ D ☐ A rồi quyết sau · ghi chú:

---

### Q-F2. Inference engine (linh hồn của SketchUp) 🔴

**Vì sao quan trọng:** Lý do SketchUp thắng là vẽ đúng không cần nhập số: tự bắt
điểm cuối, trung điểm, song song, vuông góc, trên trục, trên mặt, khóa trục bằng
phím mũi tên, nhập số trực tiếp (VCB).

**Khuyến nghị (nguyên tắc cần đồng ý):**
1. Inference là **crate riêng** (`ferro-snap`), thuần, tất định, test được, không
   phụ thuộc UI.
2. Thứ tự ưu tiên có spec: điểm (endpoint, midpoint, intersection, center) >
   cạnh/mặt > hướng (trục, song song, vuông góc, tiếp tuyến) > lưới.
3. Bắt được cả **datum BIM** (level, grid, reference plane) — thứ SketchUp không có.
4. Dung sai bắt là **pixel màn hình** (UI), không lẫn với dung sai kernel (D-06).
5. Nhập số trực tiếp mọi lúc (`3000`, `3m`, `4000,3000`, `*5` để lặp), giống VCB.
6. AI cũng dùng inference khi cần ("vẽ song song với tường A").

**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-F3. Mô hình công cụ: modal, command line, hay cả hai? 🟠

| Phương án | Được | Mất | Ai dùng |
|---|---|---|---|
| **A. Tool modal** (chọn tool rồi click) | Dễ học | Chậm với người chuyên | SketchUp |
| **B. Command line** (gõ lệnh + alias) | Cực nhanh với người chuyên | Khó học | AutoCAD, Rhino |
| **C. Cả hai + Command palette** (Ctrl+K tìm lệnh mờ) **+ ô AI** (gõ tiếng Việt tự nhiên) — cùng một hệ command | Mọi kiểu người dùng | Phải thiết kế thống nhất |

**Khuyến nghị:** **C.** Một ô nhập duy nhất: gõ `L` = lệnh Line (alias), gõ
`vẽ tường dày 200 quanh khu đất` = AI. Alias tương thích AutoCAD/SketchUp tùy chọn.
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-F4. 3D trước hay mặt bằng trước? 🟠

| Phương án | Được | Mất |
|---|---|---|
| **A. 3D trước** (SketchUp) | Trực quan, hợp concept | Kiến trúc sư VN quen suy nghĩ trên mặt bằng |
| **B. Mặt bằng trước** (Revit, AutoCAD) | Quen thuộc, chính xác | Kém cảm hứng |
| **C. Cả hai đồng thời** (chia màn hình mặt bằng + 3D, vẽ ở đâu cũng được, đồng bộ tức thì) | Tốt nhất | Hệ view phải sớm |

**Khuyến nghị:** **C**, mặc định 3D lớn + mặt bằng nhỏ. Mặt bằng là `PlanView`
(cắt 3D tại cao độ), không phải bản vẽ riêng.
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-F5. Chọn và sửa đối tượng 🟢

**Khuyến nghị:** Click chọn, kéo khung (trái→phải chọn nằm trọn, phải→trái chọn
cắt qua — chuẩn CAD), double-click vào Form để sửa bên trong (như group SketchUp),
**gizmo** di chuyển/xoay/scale kiểu Rhino Gumball, grip kéo đầu tường kiểu Revit,
thuộc tính ở panel bên. Chọn theo category/level/parameter bằng query (Q-B11).
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-F6. iPad và bút 🟠

| Phương án | Được | Mất |
|---|---|---|
| **A. Desktop trước, iPad ở milestone 5+** (cùng lõi, UI cảm ứng riêng) | Tập trung | Mất lợi thế sớm |
| **B. iPad là nền tảng hạng nhất từ đầu** | Khác biệt mạnh (Shapr3D chứng minh) | UI phải làm hai kiểu sớm |

**Khuyến nghị:** **A**, nhưng mọi quyết định lõi/render đều phải chạy được trên iOS
(không phụ thuộc thứ chỉ có trên desktop). Phác thảo bằng bút → AI dựng hình là
tính năng iPad đáng giá nhất.
**TaD trả lời:** ☐ A ☐ B · ghi chú:

---

### Q-F7. Phím tắt và chuyển đổi từ app khác 🟢

**Khuyến nghị:** Bộ phím tắt mặc định của Ferro + **preset tương thích** SketchUp,
AutoCAD, Revit để người dùng chuyển sang không phải học lại. Tùy biến hoàn toàn,
lưu theo người dùng.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-F8. Hiển thị lịch sử và hoàn tác 🟢

**Khuyến nghị:** Panel lịch sử hiển thị từng transaction (ai làm: người/AI, tên lệnh,
thời gian, ảnh thu nhỏ trước/sau). Click vào một mốc để xem trước; quay về hoặc
**tách nhánh** từ đó (Q-I3). Thay đổi của AI có nhãn riêng.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

## L. Nền tảng

### Q-L1. Thứ tự nền tảng 🟠

| Nền tảng | Đề xuất | Lý do |
|---|---|---|
| Windows | Milestone 1 | Đa số kiến trúc sư VN, Revit chỉ có Windows |
| macOS | Milestone 1 | Designer, sinh viên; Revit không có Mac → lợi thế |
| Web (xem + bình luận) | Milestone 2–3 | Gửi link cho khách hàng xem model/phương án không cần cài |
| Web (đầy đủ) | Milestone 5+ | Khi hiệu năng WebGPU đủ |
| iPad | Milestone 5 | Q-F6 |
| Linux | Theo cộng đồng | Hỗ trợ build qua CI, không ưu tiên UI |

**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-L2. Chiến lược web 🔴

**Vì sao quan trọng:** Nếu lõi lỡ dùng thứ không chạy được trên WASM (thread kiểu
desktop, file system, thư viện C++), sau này không có bản web.

| Phương án | Được | Mất |
|---|---|---|
| **A. Lõi bắt buộc build được WASM từ ngày đầu** (CI build `wasm32`) | Bản web/viewer chia sẻ luôn khả thi | Hạn chế lựa chọn thư viện (vd khó dùng OCCT) |
| **B. Desktop trước, web tính sau** | Tự do | Có thể không bao giờ có bản web |

**Khuyến nghị:** **A.** `ferro-core` phải build `wasm32-unknown-unknown` trong CI.
**Độ khó đổi:** 🔴
**TaD trả lời:** ☐ A ☐ B · ghi chú:

---

### Q-L3. Phần cứng tối thiểu 🟠

**Vì sao quan trọng:** Nhiều kiến trúc sư/sinh viên VN dùng laptop GPU tích hợp.

**Khuyến nghị:**
- API đồ họa: Metal (Mac/iPad), DX12 (Windows), Vulkan (Linux), WebGPU (web);
  dự phòng GLES/WebGL2 chỉ cho viewer.
- Mục tiêu: **nhà phố đầy đủ chạy 60 fps trên laptop GPU tích hợp 3 năm tuổi**;
  dự án lớn cần GPU rời.
- Ray tracing phần cứng: tùy chọn khi có (Q-G3), không bắt buộc.

**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:
