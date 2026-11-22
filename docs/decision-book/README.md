# Ferro Decision Book (tiếng Việt)

> Tài liệu làm việc để **founder chốt toàn bộ nền móng** của Ferro trước khi mở
> cho cộng đồng và AI đóng góp. Mỗi câu hỏi đã chốt sẽ được chuyển thành một
> quyết định `D-xx` (tiếng Anh) trong [`../foundation/`](../foundation/README.md).

## Ferro là gì (một câu)

**Một app thiết kế nhà lấy AI làm trung tâm**: người dùng đưa ý tưởng / nhiệm vụ
thiết kế, Ferro "mớm" cho họ từng bước: khối → mặt bằng → mặt đứng → vật liệu →
render → bảng trình bày → video. Mọi thứ AI tạo ra là **dữ liệu BIM thật**, nhanh
như SketchUp, chính xác như Revit, chuyển sang Revit / IFC trong một click.

## Cách đọc và cách trả lời

Mỗi câu hỏi có dạng:

```
### Q-B3. Tên câu hỏi
Vì sao quan trọng   → hậu quả nếu chọn sai
Phương án           → bảng: Được / Mất / Ai đang làm vậy
Khuyến nghị         → lựa chọn của Claude + lý do
Độ khó đổi về sau   → 🔴 rất khó · 🟠 khó · 🟢 đổi được
TaD trả lời         → ☐ A ☐ B ☐ C · ghi chú
```

Trả lời nhanh nhất: điền cột **TaD** trong [PHIEU-TRA-LOI.md](PHIEU-TRA-LOI.md)
(bảng tổng hợp mọi câu hỏi + khuyến nghị mặc định). Chỉ cần đọc kỹ những câu 🔴.

## Nhìn Ferro như một hệ điều hành

Một app thiết kế ở quy mô thị trường thực chất là một hệ điều hành thu nhỏ: có
nhân (kernel), hệ thống file, tiến trình, driver đồ họa, giao diện, ứng dụng bên
thứ ba, mạng, bảo mật. Bản đồ dưới đây là **toàn bộ những hệ con phải quyết định**:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ 10. TRẢI NGHIỆM   Shell UI · Công cụ vẽ · Inference · Command line · Panels │
│                   Chat AI · Bảng trình bày · Timeline video · Mobile/iPad   │
├─────────────────────────────────────────────────────────────────────────────┤
│ 9. AI RUNTIME     Agent · Planner · Tool (=Command) · Ngữ cảnh/nhìn model  │
│                   Nhánh đề xuất (preview) · Generative · RAG quy chuẩn     │
├───────────────┬───────────────┬───────────────┬─────────────────────────────┤
│ 8. RENDER     │ 7. PLUGIN     │ 6. CỘNG TÁC   │ 5. INTEROP                  │
│ Viewport PBR  │ WASM sandbox  │ Sync / CRDT   │ IFC · Revit · DWG · SKP     │
│ Style/NPR     │ Script        │ Lịch sử/nhánh │ 3dm · glTF/USD · Point cloud│
│ Path trace    │ Marketplace   │ Quyền / cloud │ PDF/SVG                     │
│ AI render     │ Quyền hạn     │ Comment       │                             │
│ Video         │               │               │                             │
├───────────────┴───────────────┴───────────────┴─────────────────────────────┤
│ 4. MIỀN BIM       Tường/sàn/mái · Lớp cấu tạo · Phòng/diện tích · Lỗ mở    │
│                   Cầu thang · Family · Level/Grid · Phase/Option · Site     │
├─────────────────────────────────────────────────────────────────────────────┤
│ 3. PARAMETRIC     Ràng buộc · Regeneration · Node graph · Biểu thức        │
├─────────────────────────────────────────────────────────────────────────────┤
│ 2. HÌNH HỌC       Math · Curve · B-rep half-edge · NURBS/SubD · Boolean     │
│                   2D polygon · Tham chiếu bền (topological naming)          │
├─────────────────────────────────────────────────────────────────────────────┤
│ 1. LÕI DỮ LIỆU    Document · Element · ID · Schema · Command/Transaction   │
│                   Undo · File format · Bộ nhớ · Đa luồng · Federation       │
├─────────────────────────────────────────────────────────────────────────────┤
│ 0. NỀN TẢNG       Rust · wgpu · Nền tảng (Win/Mac/iOS/Web) · License · Quản trị │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Mục lục

| Phần | File | Câu hỏi |
|---|---|---|
| 1 | [Đối thủ và bài học](01-doi-thu.md) | Bản đồ thị trường, điểm mạnh/yếu, bài học cho Ferro |
| A | [Sản phẩm, license, quản trị](02-san-pham.md) | Q-A1 … Q-A8 |
| B | [Lõi dữ liệu](03-loi-du-lieu.md) | Q-B1 … Q-B11 |
| C+D | [Hình học và parametric](04-hinh-hoc-parametric.md) | Q-C1 … Q-C8, Q-D1 … Q-D4 |
| E | [Miền BIM](05-bim.md) | Q-E1 … Q-E11 |
| F+L | [Tương tác, UI, nền tảng](06-tuong-tac-nen-tang.md) | Q-F1 … Q-F8, Q-L1 … Q-L3 |
| G+N | [Render, trình bày, video](07-render-trinh-bay.md) | Q-G1 … Q-G9, Q-N1 … Q-N4 |
| H | [AI làm trung tâm](08-ai.md) | Q-H1 … Q-H12 |
| I+J+K | [Cộng tác, interop, plugin](09-cong-tac-interop-plugin.md) | Q-I1 … Q-I5, Q-J1 … Q-J7, Q-K1 … Q-K4 |
| M+O+P | [Hiệu năng, chất lượng, bản địa hóa](10-van-hanh.md) | Q-M1 … Q-M4, Q-O1 … Q-O5, Q-P1 … Q-P3 |
| R | [Vận hành thực tế](11-van-hanh-thuc-te.md) | Q-R1 … Q-R19 |
| ✍️ | [Phiếu trả lời](PHIEU-TRA-LOI.md) | Bảng tổng hợp để điền |

## Ba nguyên tắc tao dùng để khuyến nghị

1. **AI là người dùng hạng nhất.** Mọi thứ người làm được qua chuột thì AI phải
   làm được qua command, và ngược lại. Mọi thứ AI làm phải xem trước, hoàn tác và
   truy vết được.
2. **Thứ khó đổi thì chốt sớm, thứ dễ đổi thì để thị trường quyết.** Lõi dữ liệu,
   ID, đơn vị, license, cách thay đổi model: chốt ngay. UI, style render, plugin
   marketplace: làm dần.
3. **Mượn đúng chỗ, tự làm đúng chỗ.** Tự làm những gì tạo khác biệt (lõi dữ liệu,
   AI runtime, công cụ vẽ). Dùng thư viện cho những gì đã có lời giải tốt (2D
   boolean, triangulation, font, file format).
