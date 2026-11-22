# R. Vận hành thực tế

Các phần trước chốt *nền móng*. Phần này trả lời câu hỏi "**ngày thứ Hai, một
kiến trúc sư mở Ferro thì mọi thứ chạy thế nào**": nhập file gì, thư viện có sẵn
gì, plugin cũ dùng được không, chế độ làm việc, mức chi tiết, các công cụ vẽ
hoạt động ra sao.

### Q-R1. Ma trận import / export và thứ tự làm 🟠

**Bốn mức trung thực** khi trao đổi file:

| Mức | Mang theo | Ví dụ |
|---|---|---|
| **G** Hình học | Chỉ hình | OBJ, STL |
| **GM** + vật liệu | Hình + màu/texture | glTF, FBX, SKP |
| **GMB** + dữ liệu BIM | + category, parameter, level | IFC, Revit link |
| **RT** Khứ hồi | Xuất đi, sửa bên kia, nhập lại không mất/nhân bản | Revit live link, IFC theo `Uid` |

| Định dạng | Nhập | Xuất | Mức | Đề xuất milestone |
|---|---|---|---|---|
| `.fro` / `.fro.json` | ✔ | ✔ | RT | M1 |
| OBJ, glTF/GLB | ✔ | ✔ | G/GM | M1 (xuất), M2 (nhập) |
| DXF | ✔ (nền tham chiếu, bắt điểm) | ✔ (nét mặt bằng/mặt cắt) | G | M2 |
| SKP (SketchUp) | ✔ → Form, component, vật liệu, tag→parameter | ✔ (để gửi người dùng SketchUp) | GM | M2 |
| Revit (live link qua add-in) | ✔ level, grid, tường, sàn, phòng, family, vật liệu | ✔ | RT | M3 |
| IFC4 | ✔ (plugin) | ✔ (lõi) | GMB/RT | M3 xuất, M4 nhập |
| 3DM (Rhino) | ✔ | ✔ | GM | M3 |
| E57 / LAS (point cloud) | ✔ | — | — | M3–M4 |
| PDF / ảnh (nền để vẽ lại) | ✔ | ✔ (board, bản vẽ) | — | M3 |
| OSM / bản đồ / địa hình | ✔ | — | — | M3 |
| CSV / XLSX | ✔ (Program, danh sách phòng) | ✔ (schedule) | dữ liệu | M3 |
| BCF | ✔ | ✔ | issue | M4 |
| DWG | ✔ | ✔ | G | M4 (bản Pro, D-58) |
| FBX, USD | — | ✔ | GM | M4–M5 |
| STL, 3MF (in 3D mô hình) | — | ✔ | G | M4 |

**Khuyến nghị:** theo bảng. SKP import sớm (M2) vì đó là cửa để KTS VN chuyển sang.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa thứ tự · ghi chú:

---

### Q-R2. Plugin SketchUp chạy được trong Ferro không? 🔴

**Sự thật kỹ thuật:** Plugin SketchUp (extension `.rbz`) viết bằng Ruby và gọi
trực tiếp API nội bộ của SketchUp (`Sketchup::Model`, `Entities`, `Face.pushpull`,
`UI::HtmlDialog`...). Nhiều plugin còn có phần C biên dịch sẵn, và nhiều plugin
thương mại bị mã hóa (`.rbe`) với điều khoản chỉ cho dùng trong SketchUp.

| Phương án | Được | Mất |
|---|---|---|
| **A. Lớp tương thích**: nhúng Ruby, giả lập API SketchUp để chạy plugin cũ | Người dùng giữ được plugin quen thuộc | Khối lượng khổng lồ (API SketchUp có hàng nghìn hàm); plugin mã hóa/có phần C vẫn không chạy; rủi ro pháp lý với plugin thương mại; mô hình hình học "dính" của SketchUp trái với D-12 |
| **B. Làm sẵn các công cụ phổ biến thành tính năng gốc hoặc kỹ năng AI** (Joint Push Pull, Round Corner, Curviloft, Profile Builder, 1001bit, FredoScale, Follow Me nâng cao, Solid Tools, CleanUp...) | Chạy nhanh, đúng dữ liệu BIM, AI dùng được | Không tự động có mọi plugin |
| **C. Cầu nối SketchUp ↔ Ferro**: một extension cho SketchUp đồng bộ hình học/vật liệu sang Ferro (live link) | Người dùng vẫn dùng plugin trong SketchUp trong thời gian chuyển đổi | Phải duy trì extension SketchUp |
| **D. Công cụ chuyển plugin mã nguồn mở**: AI hỗ trợ viết lại plugin Ruby mở thành plugin WASM của Ferro | Cộng đồng mang plugin sang dần | Chỉ áp dụng plugin có mã nguồn |

**Khuyến nghị:** **B + C + D, không làm A.** Lập danh sách 20 plugin SketchUp được
dùng nhiều nhất ở VN và đưa chức năng vào Ferro theo thứ tự.
**TaD trả lời:** ☐ A ☐ B ☐ C ☐ D ☐ B+C+D · ghi chú:

---

### Q-R3. Phần mềm khác link vào Ferro thế nào? 🔴

| Phương án | Được | Mất |
|---|---|---|
| **A. Một giao thức link mở duy nhất "Ferro Link"**: chính là command / query / event API (MCP qua WebSocket + luồng hình học nhị phân) chạy trên máy (và qua server khi cộng tác). Viết connector cho Rhino/Grasshopper, Blender, SketchUp, Revit, Excel, Unreal/Twinmotion/D5 trên cùng giao thức | Một API cho cả AI, plugin, app khác; tài liệu một lần; cộng đồng tự viết connector | Phải giữ giao thức ổn định (có version) |
| **B. Mỗi app một kiểu kết nối riêng** | Tối ưu từng trường hợp | Nhân đôi công sức, khó duy trì |
| **C. Chỉ qua Speckle** | Có sẵn hàng chục connector | Phụ thuộc bên thứ ba, chậm hơn live link |

**Khuyến nghị:** **A**, thêm connector Speckle như một cầu phụ.
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-R4. Tái sử dụng thư viện family Revit (.rfa) 🔴

**Sự thật kỹ thuật:** `.rfa` là định dạng đóng, chỉ đọc được qua Revit API (cần
cài Revit). Một family Revit chứa: các type + giá trị parameter, hình học theo
từng mức chi tiết (Coarse/Medium/Fine), công thức, ràng buộc, nested family.

| Phương án | Được | Mất |
|---|---|---|
| **A. Bộ chuyển đổi hàng loạt qua Revit add-in**: mở từng `.rfa` trong Revit, xuất mỗi type thành `.ffa` gồm hình học từng mức chi tiết + parameter + vật liệu + category + ảnh xem trước. Kết quả là family "đông cứng" (đúng kích thước từng type, không giữ công thức) | Mang được cả thư viện văn phòng sang trong một lần chạy | Mất tính tham số phức tạp; cần máy có Revit để chuyển |
| **B. AI tái tham số hóa**: từ các type đã chuyển, AI suy ra quy luật (rộng/cao/khung cửa) và dựng lại family có tham số cho các loại phổ biến (cửa, cửa sổ, nội thất cơ bản) | Có lại family tham số thật | Không hoàn hảo cho family phức tạp |
| **C. Chỉ dùng thư viện riêng của Ferro** | Sạch | Văn phòng mất kho family đã đầu tư nhiều năm |

**Lưu ý bản quyền:** family của hãng/nhà cung cấp có điều khoản riêng; bộ chuyển
đổi chạy trên máy người dùng cho thư viện của chính họ, Ferro không phân phối lại.
**Khuyến nghị:** **A + B.**
**TaD trả lời:** ☐ A ☐ B ☐ C ☐ A+B · ghi chú:

---

### Q-R5. Nhận vật liệu Revit 🟠

Vật liệu Revit = định danh + màu hiển thị + hatch (bề mặt/cắt) + tài sản vật lý/
nhiệt + **appearance asset** (texture, độ bóng...). Texture trong thư viện vật liệu
Autodesk có bản quyền của Autodesk.

**Khuyến nghị:** Add-in đọc vật liệu và chuyển sang `Material` của Ferro (D-08 PBR):
màu → base color, texture → base color map (giữ đường dẫn trên máy, **không** phân
phối lại texture Autodesk), độ bóng → roughness, bump → normal, hatch → pattern
2D, thông số vật lý/nhiệt giữ nguyên. Vật liệu thiếu texture thì AI đề xuất texture
tương đương từ thư viện mở.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-R6. Thư viện "mớm sẵn" khi cài Ferro 🟠

**Khuyến nghị:**
- **Ferro Standard Library** (mở, CC0/CC-BY): type tường/sàn/mái phổ biến, cửa đi/cửa
  sổ có tham số, cầu thang, lan can, nội thất cơ bản, thiết bị vệ sinh, cây, người,
  xe, vật liệu PBR cơ bản. Family **sinh bằng tham số** nên nhẹ.
- **Region pack Việt Nam** (D-63/Q-P2): tường gạch 100/200 trát hai mặt, tường BTCT,
  sàn BTCT 120 + hoàn thiện, cửa nhôm Xingfa, cửa gỗ, kích thước phổ biến, vật liệu
  của nhà cung cấp VN, template dự án (nhà phố 4×16, 5×20, biệt thự, căn hộ).
- **Gói theo hệ kết cấu / kiểu nhà** (TaD yêu cầu): mỗi gói có type tường/sàn/mái,
  family kết cấu, chi tiết, vật liệu và template riêng:
  - *Nhà kiên cố*: gạch + bê tông cốt thép (nhà phố, biệt thự VN).
  - *Nhà tiền chế thép*: khung thép, xà gồ, tôn/panel cách nhiệt (nhà xưởng, kho, nhà
    khung thép dân dụng).
  - *Nhà kiểu châu Âu*: khung thép nhẹ/khung gỗ truyền thống, tấm ốp, tấm lợp/ngói.
  - Mở rộng sau: CLT/mass timber, nhà lắp ghép module/container, tre.
- **Marketplace** (Q-G6): asset và family thương mại.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-R7. Chế độ làm việc (workspace) 🔴

**Vì sao quan trọng:** Dựng hình cần nhanh và gọn; render cần UI như Enscape/D5
(thư viện asset, cảnh, ánh sáng, timeline). Nhồi cả hai vào một giao diện thì
rối như Vectorworks; tách thành hai app thì phải export qua lại.

| Phương án | Được | Mất |
|---|---|---|
| **A. Workspace trên cùng một Document**, chuyển tức thì (phím tắt/tab), mỗi workspace có bố cục UI + chất lượng viewport + bộ công cụ riêng: **Phác thảo** (Form, khối, nhanh nhất), **Dựng hình BIM** (tường, sàn, cửa, nét, bắt điểm), **Render** (giống app render: asset, vật liệu, mặt trời, đèn, camera, cảnh, hậu kỳ, AI render, timeline video), **Trình bày** (board, diagram), **Review** (bình luận, so sánh option/nhánh) | Mỗi việc có UI tối ưu; không export; dữ liệu luôn sống | Phải thiết kế nhiều bố cục |
| **B. Một giao diện, bật/tắt tính năng** | Đơn giản | Rối khi tính năng nhiều |
| **C. Hai app riêng (Ferro + Ferro Render)** | Mỗi app gọn | Đứt quy trình, trái với "tất cả trong một" |

**Khuyến nghị:** **A.** Workspace chỉ đổi UI/chất lượng, không đổi dữ liệu. AI
biết người dùng đang ở workspace nào để gợi ý đúng việc.
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-R8. Mức chi tiết (LOD): ai quy định? 🔴

Có **hai khái niệm thường bị lẫn**:
1. **Mức hiển thị (Detail Level)**: cùng một bức tường, xem ở mức Thô chỉ là một
   khối; mức Vừa thấy lõi + lớp hoàn thiện; mức Chi tiết thấy mọi lớp, ron, gờ
   nước, lỗ thoát nước. Đây là chuyện **vẽ ra thế nào**, theo từng view.
2. **Mức phát triển (LOD 100–400 theo BIMForum)**: model đã được thiết kế đến
   đâu: LOD 100 là khối, LOD 300 là tường có type, kích thước, vật liệu đúng. Đây
   là chuyện **dữ liệu đã đủ chưa**, theo từng element.

| Phương án | Được | Mất |
|---|---|---|
| **A. Ferro quy định cứng** (3 mức hiển thị như Revit, quy tắc cố định cho từng category) | Đơn giản, nhất quán | Văn phòng không chỉnh theo chuẩn riêng được |
| **B. Người dùng tự định nghĩa hoàn toàn** | Linh hoạt tối đa | Mỗi văn phòng một kiểu, AI và plugin không biết dựa vào đâu |
| **C. Mặc định của Ferro + văn phòng/người dùng ghi đè**: 3 mức hiển thị (Thô/Vừa/Chi tiết) chọn theo view; mỗi category/family có **quy tắc hiển thị mặc định** (tường: Thô = một khối, Vừa = lõi + hoàn thiện, Chi tiết = mọi lớp + chi tiết như ron, gờ nước, lỗ thoát nước); quy tắc chỉnh được trong view template và template dự án. **Mức phát triển LOD** là parameter riêng của element, có bộ kiểm tra "LOD 300 cần những gì" | Chạy được ngay, văn phòng vẫn theo chuẩn riêng; AI có quy tắc rõ để nâng cấp model (LOD 100 → 300) | Phải thiết kế hệ quy tắc |

**Khuyến nghị:** **C.** Mức hiển thị còn dùng cho renderer (ở xa tự hạ mức, Q-C7).
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-R9. System family và loadable family? 🔴

Đã ngầm chốt ở D-14/D-43; câu này để chốt rõ ranh giới.

| Loại | Gồm | Tạo/sửa thế nào | Revit |
|---|---|---|---|
| **System family** (có sẵn trong Ferro) | Tường, sàn, mái, trần, cầu thang, lan can, vách kính (curtain wall), level, grid, phòng, ram dốc | Không nạp từ file; người dùng tạo/sửa **type** (lớp cấu tạo, kích thước), chuyển type giữa các dự án | System family + Transfer Project Standards |
| **Loadable family** (`.ffa`) | Cửa đi, cửa sổ, nội thất, thiết bị, cây, đèn, cột/dầm định hình, chi tiết | Nạp từ thư viện, AI sinh, chuyển từ `.rfa` | Loadable family `.rfa` |
| **Form / component** | Hình tự do trong dự án; có thể biến thành component dùng lại (Q-R10) | Vẽ trực tiếp | In-place family / generic model |

**Khuyến nghị:** **Đúng ba loại như trên.** Plugin có thể đăng ký system family mới
(vd hệ lam mặt tiền tham số) qua schema mở rộng (D-36).
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-R10. Component kiểu SketchUp (sửa một, tất cả cập nhật) 🟠

**Khuyến nghị:** Một Form có thể "biến thành component": tạo ra một **Form Type**
(định nghĩa) + các instance; sửa định nghĩa thì mọi instance cập nhật, có thể
"make unique" như SketchUp. Về bản chất đây là loadable family nhúng trong dự án,
nên xuất ra `.ffa` được để dùng lại ở dự án khác. Không có khái niệm "group" riêng
(group = component chỉ có một instance).
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-R11. Kiến trúc chung của một công cụ (tool) 🔴

**Khuyến nghị: mọi công cụ theo cùng một khuôn:**

```
Rảnh ──click/nhập──► Đang chọn điểm ──► Xem trước ──commit──► Rảnh
   ▲                     │  ▲              │
   └──── Esc hủy ────────┘  └─ Backspace ──┘  (lùi một bước)
```

1. Công cụ là **máy trạng thái** trong app (không ở lõi), dùng inference
   (D-60) để lấy điểm.
2. **Xem trước** = mở `TransactionGroup`, chạy command, hiển thị, rollback liên tục
   theo chuột; **commit** = một undo step (D-17, Q-B6).
3. Ô nhập số (VCB) luôn sẵn sàng; phím mũi tên khóa trục; Ctrl/Alt đổi biến thể
   (copy, từ tâm...) giống SketchUp.
4. Mỗi công cụ **kết thúc bằng một command có tên** (`create_rectangle`,
   `push_pull_face`...) nên AI làm được y hệt.
5. Công cụ hiển thị gợi ý trạng thái ở thanh dưới (như SketchUp) và trong AI panel.

**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-R12. Push-pull trên element BIM 🔴

**Vì sao quan trọng:** Đây là chỗ "SketchUp gặp Revit". Người dùng kéo mặt ngoài
của một bức tường Revit-style thì chuyện gì xảy ra?

| Phương án | Được | Mất |
|---|---|---|
| **A. Push-pull hiểu ngữ nghĩa**: kéo mặt bên tường → đổi **vị trí đường định vị** (tường dịch ra) hoặc **độ dày** nếu giữ Alt (đổi/ tạo type mới); kéo mặt trên → đổi **top offset**; kéo mặt sàn → đổi cao độ/độ dày; kéo mặt cửa sổ → đổi rộng/cao. Mọi thay đổi là đổi **parameter**, không phá element | Cảm giác SketchUp, dữ liệu vẫn là BIM sạch | Phải định nghĩa ánh xạ "mặt → parameter" cho mỗi category |
| **B. Push-pull biến element thành Form** | Đơn giản | Mất dữ liệu BIM ngay lần kéo đầu |
| **C. Không cho push-pull element BIM** | An toàn | Mất trải nghiệm cốt lõi |

**Khuyến nghị:** **A**, dựa trên tham chiếu ngữ nghĩa (D-25). Khi người dùng muốn
hình dạng không biểu diễn được bằng parameter (vd khoét một góc tường), Ferro hỏi:
"tạo phần cắt (void) hay chuyển thành Form?"
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-R13. Bộ công cụ milestone 1–2 🟠

**Khuyến nghị:**

| Nhóm | Công cụ |
|---|---|
| Điều hướng | Orbit, Pan, Zoom, Zoom extents, View presets (top/front/iso), Walk |
| Chọn | Select (click, khung trái/phải), chọn theo category/level, Isolate/Hide |
| Phác (Form) | Line, Rectangle, Circle, Arc, Polygon, Push/Pull, Offset, Follow Me, Eraser, Tape/Guide |
| Biến đổi | Move/Copy (+ array `*n`, `/n`), Rotate, Scale, Mirror, Align |
| BIM | Level, Grid, Wall (vẽ hoặc chọn đường/mặt Form), Floor, Roof (theo đường viền + độ dốc), Door/Window, Room, Opening |
| Dữ liệu | Paint (vật liệu), Properties panel, Type editor |
| Xem | Section plane (cắt 3D), Plan view, Style, Sun |
| Chuyển đổi | Form → Walls/Floors/Roof, Lines → Walls, CAD underlay → Walls (AI hỗ trợ) |

**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-R14. Layer / tag / ẩn hiện 🟠

| Phương án | Được | Mất |
|---|---|---|
| **A. Không có layer kiểu CAD**: ẩn hiện theo category + **bộ lọc theo parameter** + **Tag** (một parameter nhiều giá trị người dùng tự đặt, giống tag SketchUp) + workset khi cộng tác | Dữ liệu sạch, không có element "lạc layer"; tag SketchUp nhập vào thành Tag | Dân CAD phải quen cách mới |
| **B. Có layer như AutoCAD/SketchUp cũ** | Quen thuộc | Trùng lặp với category, sinh dữ liệu bẩn |

**Khuyến nghị:** **A.**
**TaD trả lời:** ☐ A ☐ B · ghi chú:

---

### Q-R15. Texture trên Form 🟢

**Khuyến nghị:** Mặc định ánh xạ theo **kích thước thật** của vật liệu (gạch
600×600 thì ô gạch đúng 600), canh theo trục thế giới hoặc theo mặt; chỉnh từng mặt
(dịch, xoay, scale) như "Texture Position" của SketchUp; giữ khi push-pull.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-R16. Lưu tự động và khôi phục 🟢

**Khuyến nghị:** Nhờ SQLite (D-21), mỗi commit ghi xuống đĩa trong vài giây (không
cần nút Save "sống còn"); mất điện mở lại đúng trạng thái cuối; lịch sử phiên bản
giữ theo ngày; nút Save vẫn có để tạo mốc có tên.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-R17. Nền CAD và file nhập 🟢

**Khuyến nghị:** DWG/DXF/PDF nhập vào mặc định là **link tham chiếu** (không phá
ra nét rời), bắt điểm được, đặt theo level, ẩn hiện theo layer gốc của file CAD (chỉ
trong link). Có lệnh "dựng tường từ nét CAD" (AI + heuristic). SKP nhập vào thành
Form/component sửa được.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-R18. Copy/paste giữa dự án và giữa app 🟢

**Khuyến nghị:** Copy element giữa các dự án Ferro (kèm type/vật liệu, tự xử lý
trùng tên); paste bảng Excel thành Program/danh sách phòng; paste ảnh thành tham
chiếu; paste hình học từ app khác qua clipboard (OBJ/glTF) khi hệ điều hành cho phép.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-R19. Template dự án và lần mở đầu tiên 🟢

**Khuyến nghị:** Màn hình bắt đầu chọn template (`.fte`: nhà phố, biệt thự, căn hộ,
quán cà phê, trống) hoặc "Bắt đầu bằng AI" (mô tả công trình → AI tạo Brief +
Program + Site). Dự án mẫu có sẵn để học. Văn phòng tạo template riêng.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:
