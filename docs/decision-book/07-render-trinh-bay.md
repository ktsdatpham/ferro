# G. Render · N. Trình bày, phân tích, video

Ferro muốn thay luôn chuỗi "SketchUp → Enscape/Lumion → Photoshop → InDesign →
Premiere" bằng một app. Phần này quyết định chuỗi đó được xây trên nền gì.

## G. Render

### Q-G1. Engine render: tự viết hay dùng engine có sẵn? 🔴

| Phương án | Được | Mất | Ai dùng |
|---|---|---|---|
| **A. Renderer riêng trên wgpu** (`ferro-render`), thiết kế quanh Document snapshot | Kiểm soát hoàn toàn, nhẹ, chạy web/iPad, tối ưu riêng cho kiến trúc (nhiều mặt phẳng, nét viền, instancing) | Tốn công làm PBR, bóng, GI | SketchUp, Shapr3D (riêng) |
| **B. Dùng Bevy** (engine game Rust có PBR, ECS) | Có ngay PBR, bóng, post-process, cộng đồng lớn | Bevy thay API thường xuyên, kéo theo kiến trúc ECS vào app, khó tách | Một số tool Rust |
| **C. Nhúng engine khác** (Filament của Google, Godot) | Chất lượng sẵn | Ngôn ngữ khác, tích hợp khó |
| **D. Không render đẹp trong app, live link sang Unreal/Twinmotion/D5/Enscape** | Không phải làm | Mất trải nghiệm "tất cả trong một", phụ thuộc đối tác |

**Khuyến nghị:** **A**, mượn có chọn lọc các crate độc lập (không mượn cả Bevy).
Kiến trúc render theo **pass có thể ghép** (render graph), để style trình bày
(Q-G4), AI render (Q-G7) và path tracing (Q-G3) đều là cấu hình của cùng một hệ.
Vẫn làm **xuất sang engine ngoài** (glTF/USD, Q-J6) cho ai cần.
**Độ khó đổi:** 🔴
**TaD trả lời:** ☐ A ☐ B ☐ C ☐ D · ghi chú:

---

### Q-G2. Chất lượng viewport real-time 🟠

**Khuyến nghị: ba cấp chất lượng, chuyển tức thì:**

| Cấp | Dùng khi | Kỹ thuật |
|---|---|---|
| **Làm việc** | Đang vẽ, model lớn, máy yếu | Shading đơn giản, nét viền, bóng mặt trời cơ bản, 60 fps bắt buộc |
| **Trình bày** | Xem với khách, chụp nhanh | PBR, bóng mềm (cascaded shadow maps), AO (GTAO), phản chiếu màn hình, bầu trời vật lý + IBL, TAA — mức Enscape/Twinmotion |
| **Cuối cùng** | Ảnh xuất | Path tracing tích lũy (Q-G3) hoặc AI render (Q-G7) |

Ánh sáng mặt trời **luôn đúng vị trí địa lý và giờ** (từ `Site`, Q-E10).
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-G3. Ảnh chất lượng cao: path tracing 🟠

| Phương án | Được | Mất |
|---|---|---|
| **A. Path tracer GPU trong app** (BVH trên compute shader; dùng ray tracing phần cứng khi có) | Ảnh cuối không rời app, cùng vật liệu, cùng scene | Công sức lớn, nhiễu cần denoiser (vd OIDN) |
| **B. Xuất sang Cycles/V-Ray/Corona** | Chất lượng đỉnh | Rời app |
| **C. Render cloud** (Ferro Cloud chạy path tracer trên GPU server) | Máy yếu cũng có ảnh đẹp; nguồn thu | Vận hành server |

**Khuyến nghị:** **A + C** dùng chung một path tracer (chạy local hoặc trên cloud);
**B** qua xuất USD/glTF. Milestone 5.
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-G4. Style trình bày (NPR) 🔴

**Vì sao quan trọng:** Kiến trúc sư VN dùng SketchUp phần lớn vì **style**: nét
phác tay, model trắng, line art, màu nước, diagram. Style cũng là thứ làm nên
"chất" bộ hồ sơ ý tưởng.

| Phương án | Được | Mất |
|---|---|---|
| **A. Style là element dữ liệu** (`RenderStyle`): ghép từ các lớp: **nét** (viền ngoài, cạnh gãy, nét ẩn, rung tay, đầu nét kéo dài), **tô** (vật liệu / đơn sắc / theo parameter / theo phase / theo option), **hiệu ứng** (giấy, màu nước, đổ bóng, sương), **nền** (trời, màu, ảnh) | Người dùng tự tạo, chia sẻ, AI tạo style từ mô tả hoặc ảnh mẫu; style dùng chung cho viewport, ảnh, board, video | Phải thiết kế hệ shader ghép được |
| **B. Một số style cố định** | Nhanh | Không phân biệt được với đối thủ |

**Khuyến nghị:** **A.** Thêm **tô màu theo dữ liệu** (theo công năng phòng, theo
diện tích, theo độ cao) — biến dữ liệu BIM thành diagram trình bày ngay lập tức,
thứ SketchUp không có.
**Độ khó đổi:** 🟠
**TaD trả lời:** ☐ A ☐ B · ghi chú:

---

### Q-G5. Chuẩn vật liệu appearance 🟠

| Phương án | Được | Mất |
|---|---|---|
| **A. glTF PBR (metallic-roughness) + extension** | Chuẩn web/AR, mọi engine đọc được | Hạn chế với vật liệu phức tạp (vải, da, SSS) |
| **B. OpenPBR / MaterialX** | Chuẩn điện ảnh/thiết kế mới, chính xác | Phức tạp hơn, hỗ trợ real-time đang phát triển |
| **C. Tự định nghĩa** | Tự do | Không trao đổi được |

**Khuyến nghị:** **A làm lõi, ánh xạ sang B khi xuất** (USD/MaterialX).
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-G6. Thư viện asset (cây, người, xe, nội thất, vật liệu) 🟠

**Vì sao quan trọng:** Đây là lý do người ta trả tiền cho Lumion/D5/Enscape.

| Phương án | Được | Mất |
|---|---|---|
| **A. Thư viện mở** (asset CC0 như Poly Haven, cộng đồng đóng góp) | Miễn phí, hợp tinh thần mở | Chất lượng/độ phủ không đều |
| **B. Thư viện thương mại** (Ferro Cloud, mua/thuê) | Nguồn thu, chất lượng | Phải sản xuất/mua bản quyền |
| **C. Marketplace** (nhà cung cấp VN đưa sản phẩm thật: gạch, sơn, đèn, nội thất có giá) | Asset thật + dữ liệu thật + nguồn thu quảng cáo/hoa hồng | Cần thương lượng đối tác |
| **D. AI sinh asset 3D** (từ ảnh/mô tả) | Vô hạn | Chất lượng, bản quyền chưa chắc chắn |

**Khuyến nghị:** **A + C** là chiến lược, **D** như tính năng; mọi asset có
**license gắn kèm** (dữ liệu bắt buộc) và LOD cho instancing. Rải cây/cỏ
(scattering) là tính năng lõi của renderer.
**TaD trả lời:** ☐ A ☐ B ☐ C ☐ D · ghi chú:

---

### Q-G7. AI render (diffusion) 🔴

**Vì sao quan trọng:** Ở giai đoạn ý tưởng, AI render đang thay thế render truyền
thống. Nhưng AI hay "bịa" hình học: đổi số tầng, mất cửa sổ.

| Phương án | Được | Mất |
|---|---|---|
| **A. Renderer xuất các bộ đệm điều khiển** (depth, normal, đường nét, mask theo vật liệu/category/element, ánh sáng) làm đầu vào cho mô hình diffusion có điều khiển → **giữ đúng hình học**, cho phép sửa từng vùng (chỉ đổi vật liệu mặt tiền) | Kết quả đáng tin, chỉnh được, nhất quán giữa các góc | Phải làm hệ AOV và tích hợp mô hình |
| **B. Gửi ảnh chụp màn hình cho AI** | Dễ | Không kiểm soát |
| **C. Chỉ xuất ảnh, dùng tool ngoài** | Không phải làm | Mất trải nghiệm |

**Khuyến nghị:** **A**, với **giao diện provider** để cắm: mô hình chạy local (cho
người có GPU, riêng tư), Ferro Cloud, hoặc dịch vụ bên thứ ba. Ảnh AI luôn lưu kèm
view, style, seed, prompt → làm lại được (tất định trong khả năng của provider).
**Độ khó đổi:** 🟠
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-G8. Video và animation 🟠

**Khuyến nghị (kiến trúc dữ liệu):**
- `Timeline` element chứa các track: **camera** (keyframe, đường đi, tốc độ),
  **mặt trời** (giờ trong ngày, mùa), **phase** (xem quá trình xây), **option**
  (chuyển phương án), **element** (cửa mở, xe chạy, người đi), **style** (chuyển
  từ phác tay sang render thật).
- Xuất video bằng renderer real-time (nhanh) hoặc path tracer (đẹp), encode
  H.264/H.265/AV1.
- **AI video**: dùng khung hình đã render làm điều khiển cho mô hình image-to-video
  (provider như Q-G7).
- AI tự dựng video từ mô tả ("video 30 giây đi từ cổng vào phòng khách lúc chiều").

**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-G9. Render dự án lớn 🟠

**Khuyến nghị (nguyên tắc):** GPU-driven rendering (culling trên GPU, draw
indirect), instancing bắt buộc, frustum + occlusion culling, LOD theo khoảng cách
(Q-C7), stream dữ liệu theo vùng nhìn, tọa độ tương đối camera (D-05). Hình học ảo
kiểu Nanite là nghiên cứu dài hạn. Mục tiêu số liệu ở Q-M1.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

## N. Trình bày, phân tích, xuất bản

### Q-N1. Bảng trình bày (presentation board) 🟠

| Phương án | Được | Mất | Ai làm |
|---|---|---|---|
| **A. Board là loại tài liệu trong Ferro** (khổ giấy, lưới, khung chứa view sống, ảnh render, diagram, text, logo; template; xuất PDF/PNG) | **View trên board tự cập nhật khi model đổi**; AI dàn trang tự động từ template | Phải làm một "InDesign mini" | SketchUp LayOut, Vectorworks |
| **B. Xuất ảnh, dàn trang ở app khác** | Không phải làm | Đứt quy trình, mất "mớm cho ăn" |

**Khuyến nghị:** **A**, ở milestone 4. Template board theo phong cách văn phòng
(chia sẻ được), AI "tạo bộ trình bày ý tưởng 6 trang" là demo bán hàng chính.
**TaD trả lời:** ☐ A ☐ B · ghi chú:

---

### Q-N2. Diagram sinh từ dữ liệu 🟢

**Khuyến nghị:** Bộ lệnh sinh diagram từ model: axonometric tách tầng (exploded),
diagram công năng tô màu, diagram giao thông, diagram khối/diện tích theo tầng,
đường đi nắng, diagram vật liệu. Mỗi diagram = một `View` + `RenderStyle` + chú
thích tự động, sống theo model.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-N3. Phân tích (nắng, ánh sáng, năng lượng) 🟠

| Phân tích | Đề xuất |
|---|---|
| Bóng đổ / giờ nắng | Trong lõi (dùng renderer), milestone 3 |
| Chiếu sáng tự nhiên (daylight factor) | Plugin, dùng path tracer |
| Tầm nhìn, hướng gió đơn giản | Plugin |
| Năng lượng | Xuất sang OpenStudio/EnergyPlus (mở) qua plugin |

**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-N4. Xuất bản vẽ 2D 🟠

Liên quan Q-A1. **Khuyến nghị:** Sớm (milestone 3) xuất **nét mặt bằng/mặt cắt/mặt
đứng** (hidden-line, đúng nét cắt/nét thấy) ra **DXF, SVG, PDF** để người dùng đưa
vào AutoCAD/Revit làm tiếp. Kích thước và chú thích tự động làm sau, khi quyết
định B ở Q-A1.
**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:
