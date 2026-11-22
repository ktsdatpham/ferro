# M. Hiệu năng · O. Chất lượng · P. Bản địa hóa

## M. Hiệu năng và quy mô

### Q-M1. Ngân sách hiệu năng 🔴

**Vì sao quan trọng:** "Nhanh hơn Revit" phải là con số đo được trong CI, không
phải khẩu hiệu. Không có ngân sách, app chậm dần qua từng tính năng.

**Khuyến nghị (mục tiêu ban đầu, máy tham chiếu: laptop GPU tích hợp 3 năm tuổi):**

| Chỉ số | Nhà phố (~2k element) | Chung cư 20 tầng (~100k element) |
|---|---|---|
| Khung hình khi xoay | 60 fps | 30 fps (60 fps trên GPU rời) |
| Phản hồi công cụ vẽ (push-pull kéo) | < 16 ms | < 16 ms |
| Commit một lệnh đơn | < 10 ms | < 50 ms |
| Mở file | < 1 s | < 5 s |
| Lưu (tăng dần) | < 100 ms | < 500 ms |
| Undo | < 50 ms | < 100 ms |
| AI: trả về đề xuất đầu tiên | < 5 s (không tính thời gian mô hình suy nghĩ) | |

CI chạy benchmark và **chặn PR làm chậm hơn 10%**.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa con số · ghi chú:

---

### Q-M2. Quy mô tối đa thiết kế cho 🔴

| Phương án | Ý nghĩa |
|---|---|
| **A. Nhà ở, công trình vừa (≤ 20k element/file)** | Đủ cho đa số dự án VN; đơn giản |
| **B. Công trình lớn (≤ 500k element/file) + federation** | Bệnh viện, cao tầng, khu phức hợp |
| **C. Đô thị/hạ tầng (hàng triệu element, nhiều km)** | Ngoài phạm vi |

**Khuyến nghị:** **Kiến trúc cho B, tối ưu trước cho A.** C không làm (giới hạn
10 km D-05).
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-M3. Bộ nhớ 🟠

**Khuyến nghị:** Ngân sách RAM: nhà phố < 500 MB, chung cư 100k element < 4 GB.
Tessellation và texture là cache có thể giải phóng; lịch sử undo cũ nén/đẩy xuống
đĩa (SQLite).
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-M4. Đo đạc hiệu năng liên tục 🟢

**Khuyến nghị:** `tracing` cho mọi transaction/regeneration/frame; bộ model mẫu cố
định cho benchmark; báo cáo hiệu năng tự động ở mỗi PR; công cụ profile tích hợp
trong bản dev.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

## O. Chất lượng và phát hành

### Q-O1. Chiến lược kiểm thử 🟠

**Khuyến nghị:**
- Unit test + **property-based test** (toán, hình học: "kéo rồi kéo ngược phải về
  như cũ").
- **Model vàng** (golden): bộ file mẫu, kết quả lệnh so với snapshot.
- **Test hình ảnh** (render so sánh pixel có ngưỡng) cho renderer và style.
- **Fuzz** mọi bộ đọc file (IFC, DXF, SKP, `.ferro`) — file người dùng là đầu vào
  không tin cậy.
- `Solid::validate()` sau mọi thao tác trong test.
- Ferro-Bench cho AI (Q-H12).
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-O2. Telemetry và báo lỗi 🟠

| Phương án | Được | Mất |
|---|---|---|
| **A. Tắt mặc định, người dùng bật (opt-in)**, chỉ số liệu ẩn danh + báo crash, không bao giờ gửi model | Tôn trọng riêng tư, hợp mã nguồn mở | Ít dữ liệu |
| **B. Bật mặc định (opt-out)** | Nhiều dữ liệu cải tiến | Mất lòng tin |

**Khuyến nghị:** **A.**
**TaD trả lời:** ☐ A ☐ B · ghi chú:

---

### Q-O3. Phiên bản và nhịp phát hành 🟢

**Khuyến nghị:** Semver; ba kênh **nightly / beta / stable**; stable mỗi 6–8 tuần
sau bản 1.0; changelog bắt buộc; MCP tool schema và plugin API có version riêng và
cam kết tương thích.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-O4. An ninh 🟠

**Khuyến nghị:** `SECURITY.md` và kênh báo lỗ hổng riêng; `cargo-deny`/`cargo-audit`
kiểm tra license và lỗ hổng phụ thuộc trong CI; ký số bản phát hành (Windows code
signing, Apple notarization); bản build tái lập được; plugin ký số (Q-K3).
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-O5. Tài liệu 🟢

**Khuyến nghị:** Tài liệu API tự sinh (rustdoc); trang docs người dùng **song ngữ
Việt/Anh**; hướng dẫn plugin; spec nền tảng (đang làm); video hướng dẫn trên kênh
YouTube TaD Design.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

## P. Bản địa hóa và tiêu chuẩn

### Q-P1. Đa ngôn ngữ 🟠

**Khuyến nghị:** UI tiếng Việt + tiếng Anh từ bản đầu tiên (Project Fluent cho
dịch); mọi chuỗi UI qua hệ dịch; AI trả lời theo ngôn ngữ người dùng; tên parameter
built-in giữ tiếng Anh (khóa ổn định), nhãn hiển thị dịch được.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-P2. Gói khu vực (region pack) 🟠

**Khuyến nghị:** Một "region pack" gom mọi thứ phụ thuộc quốc gia: đơn vị hiển thị,
quy cách nét/hatch theo TCVN, cách tính diện tích (Q-E4), quy chuẩn (Q-E11), gói tri
thức (Q-H9), template board, family/vật liệu phổ biến. **Gói Việt Nam là gói đầu
tiên và mẫu cho các nước khác.**
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-P3. Trợ năng và giao diện 🟢

**Khuyến nghị:** Theme sáng/tối, chỉnh cỡ chữ UI, điều khiển bằng bàn phím đầy đủ,
không dùng màu làm tín hiệu duy nhất (người mù màu), hỗ trợ màn hình độ phân giải
cao và nhiều màn hình.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:
