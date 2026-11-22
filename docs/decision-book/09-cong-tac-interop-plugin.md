# I. Cộng tác · J. Interop · K. Plugin

## I. Cộng tác

### Q-I1. Mô hình cộng tác 🔴

**Vì sao quan trọng:** Quyết định cách đồng bộ dữ liệu, ảnh hưởng ngược lại lõi
dữ liệu (Q-B1, Q-B3) và ID. Đồ án lớn không thể một người làm.

| Phương án | Được | Mất | Ai dùng |
|---|---|---|---|
| **A. File đơn, gửi qua lại** | Đơn giản | Không phải cộng tác | SketchUp |
| **B. Model trung tâm + khóa element** (mượn element để sửa, đồng bộ khi bấm) | Đã chứng minh trong BIM, an toàn | Không real-time, xung đột khó chịu | Revit worksharing, ArchiCAD Teamwork |
| **C. Real-time kiểu Figma**: local-first, mỗi máy gửi thay đổi dạng lệnh/kết quả lên server; server xếp thứ tự; xung đột giải ở mức **thuộc tính của element** (người sau thắng) + khóa mềm khi đang sửa hình học | Thấy người khác làm ngay, làm offline được, trải nghiệm hiện đại | Thiết kế kỹ, cần server |
| **D. CRDT đầy đủ** (kể cả hình học) | Không cần server trung tâm | Hình học B-rep không hợp CRDT; phức tạp, dễ sinh trạng thái vô nghĩa | Nghiên cứu |

**Khuyến nghị:** **Thiết kế lõi cho C** (Document bất biến + nhật ký lệnh + `Uid`
đã sẵn sàng), **ra mắt bản đơn người dùng trước**, cộng tác ở milestone 4. Đồng bộ
**kết quả** (element đã đổi), không phát lại lệnh (Q-B8).
**Độ khó đổi:** 🔴
**TaD trả lời:** ☐ A ☐ B ☐ C ☐ D · ghi chú:

---

### Q-I2. Server cộng tác 🟠

| Phương án | Được | Mất |
|---|---|---|
| **A. Server mã nguồn mở tự host được + Ferro Cloud bản hosted** | Văn phòng VN/doanh nghiệp tự host (dữ liệu trong nước); có nguồn thu | Phải duy trì cả hai |
| **B. Chỉ Ferro Cloud** | Dễ kiểm soát | Doanh nghiệp e ngại, mâu thuẫn tinh thần mở |

**Khuyến nghị:** **A** (giống GitLab, Penpot).
**TaD trả lời:** ☐ A ☐ B · ghi chú:

---

### Q-I3. Phiên bản và nhánh (git cho công trình) 🔴

**Khuyến nghị:** Có từ lõi: tạo nhánh từ bất kỳ mốc nào, so sánh hai phiên bản (diff
theo element, xem trực quan bằng màu), gộp nhánh (merge theo element; xung đột
hiển thị để người chọn), **đề xuất thay đổi** (giống pull request) cho cả người và
AI (Q-H5). Đây là thứ Revit/ArchiCAD không có và đội thiết kế rất cần.
**Độ khó đổi:** 🔴
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-I4. Phân quyền 🟠

**Khuyến nghị:** Vai trò theo dự án (chủ, sửa, bình luận, xem), phạm vi theo bộ môn
hoặc vùng (tầng, tòa). **Agent AI là một chủ thể có quyền riêng** (vd agent của
khách hàng chỉ được bình luận; agent kiểm tra chỉ được đọc).
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-I5. Bình luận và xử lý issue 🟢

**Khuyến nghị:** Bình luận gắn vào element/view/điểm trong không gian, có ảnh chụp,
trạng thái (mở/đã xử lý), giao cho người/agent. Nhập/xuất chuẩn **BCF** để làm việc
với Revit, Navisworks, Solibri. Khách hàng bình luận qua link web (Q-L1).
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

## J. Interop

### Q-J1. IFC 🔴

| Phương án | Được | Mất |
|---|---|---|
| **A. Tự viết bộ đọc/ghi IFC bằng Rust** | Thuần Rust, chạy web, license sạch | Đọc IFC đầy đủ rất nặng (hình học IFC đa dạng) |
| **B. Dùng IfcOpenShell** (C++, LGPL) | Chuẩn mực, đầy đủ nhất | C++, khó build web/iOS |
| **C. Dùng web-ifc** (C++ → WASM, MPL) | Chạy được web, nhanh | C++ |
| **D. Ghi IFC tự viết (dễ hơn đọc) + đọc IFC qua C trong plugin/sidecar** | Xuất IFC sạch sớm; nhập IFC đầy đủ vẫn có | Hai đường |

**Khuyến nghị:** **D.** Mục tiêu IFC4 (ADD2 TC1) chứng nhận được về sau, theo dõi IFC5.
**TaD trả lời:** ☐ A ☐ B ☐ C ☐ D · ghi chú:

---

### Q-J2. Revit 🔴

**Khuyến nghị:**
- Add-in C# mỏng + lõi Rust (đã có).
- **Live link** qua kết nối local (socket/named pipe), không qua file: sửa trong
  Ferro → Revit cập nhật trong vài giây, khớp theo `Uid` (D-13).
- **Hai chiều**: nhập từ Revit level, grid, tường, sàn, phòng của dự án có sẵn vào
  Ferro để thiết kế lại/cải tạo; đẩy ngược về.
- Hỗ trợ 3 phiên bản Revit gần nhất.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-J3. DWG 🔴

**Vì sao quan trọng:** Mọi văn phòng VN đều có DWG (mặt bằng hiện trạng, quy hoạch).
DWG là định dạng đóng.

| Phương án | Được | Mất |
|---|---|---|
| **A. Open Design Alliance (ODA) SDK** | Đầy đủ, chuẩn | Phí thành viên hằng năm, mã đóng, khó với mã nguồn mở |
| **B. LibreDWG** | Mở | **GPL** → không phân phối chung với lõi MPL/Apache; chất lượng chưa bằng |
| **C. DXF tự viết** (định dạng mở) + DWG qua dịch vụ/plugin | Lõi sạch license | Người dùng phải lưu DXF hoặc dùng plugin |

**Khuyến nghị:** **C**: DXF đọc/ghi trong lõi; DWG qua plugin dùng ODA trong bản
Ferro Pro/Cloud (thương mại, tách khỏi lõi mở).
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-J4. SketchUp `.skp` 🟠

**Vì sao quan trọng:** Đa số kiến trúc sư VN có kho model SketchUp; nhập được SKP
là cửa để họ chuyển sang.

**Khuyến nghị:** Plugin nhập SKP dùng SketchUp SDK chính thức (miễn phí, mã đóng,
cần kiểm tra điều khoản phân phối), chuyển group/component thành Form, tag/layer
thành parameter, vật liệu thành Material. Không đưa SDK vào lõi.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-J5. Rhino `.3dm` 🟢

**Khuyến nghị:** Đọc/ghi qua openNURBS (license thoáng, miễn phí). Dễ, nên làm sớm.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-J6. glTF và USD 🟢

**Khuyến nghị:** **glTF** xuất sớm (web, AR, xem trên điện thoại, đưa vào engine
khác). **OpenUSD** sau (Omniverse, Blender, Unreal, chuỗi render chuyên nghiệp).
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-J7. Kết nối hệ sinh thái khác 🟢

**Khuyến nghị:** Connector **Speckle** (đi tắt sang hàng chục app), xuất schedule
CSV/Excel, nhập PDF/ảnh làm nền để vẽ lại, nhập point cloud E57/LAS (Q-C8).
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

## K. Plugin

### Q-K1. Plugin chạy bằng gì? 🔴

| Phương án | Được | Mất | Ai dùng |
|---|---|---|---|
| **A. Thư viện native (dll/dylib)** | Nhanh nhất | Không an toàn (plugin độc chiếm cả máy), ABI Rust không ổn định, phải build cho từng nền tảng | Revit, SketchUp (C ext) |
| **B. WASM sandbox** (wasmtime, component model) | An toàn, một bản chạy mọi nền tảng kể cả web, viết bằng nhiều ngôn ngữ | Hiệu năng thấp hơn native một chút, công cụ còn mới | Figma, Zed, Shopify |
| **C. Script (Python/JS)** | Dễ viết | Khó sandbox (Python), chậm | Blender, SketchUp (Ruby) |

**Khuyến nghị:** **B làm chuẩn phân phối**, script Python cho tự động hóa cá nhân
trên desktop (Q-D4).
**Độ khó đổi:** 🔴
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-K2. Plugin được mở rộng những gì? 🟠

**Khuyến nghị — các điểm mở rộng chính thức:** command mới, công cụ vẽ, panel UI,
category con/parameter/schema (Q-B2), bộ quy tắc kiểm tra (Q-E11), area scheme
(Q-E4), kỹ năng AI và bộ đánh giá (Q-H4, Q-H6), gói tri thức (Q-H9), render style
(Q-G4), provider AI render/video (Q-G7), importer/exporter, generator node (Q-D2).
API có version theo semver; plugin khai báo phiên bản API cần.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-K3. Quyền hạn và an ninh plugin 🔴

**Khuyến nghị:** Mô hình quyền tường minh (giống app điện thoại): đọc model / sửa
model / mạng (tên miền nào) / file / gọi AI (tốn tiền của người dùng). Người dùng
cấp quyền khi cài. Plugin trong kho chính thức phải ký số. Mọi thay đổi của plugin
đi qua command → có trong lịch sử, hoàn tác được.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-K4. Phân phối plugin 🟢

**Khuyến nghị:** Giai đoạn đầu: registry dựa trên Git (miễn phí). Sau: marketplace
trong app, cho phép plugin trả phí (chia doanh thu), có xếp hạng và review.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:
