# E. Miền BIM

### Q-E1. Hệ phân loại gốc: kiểu Revit, IFC gốc, hay tự định nghĩa? 🔴

**Vì sao quan trọng:** Đây là "từ vựng" của toàn bộ model. Bonsai chọn IFC làm dữ
liệu gốc; Revit dùng category riêng rồi ánh xạ ra IFC.

| Phương án | Được | Mất | Ai dùng |
|---|---|---|---|
| **A. Category kiểu Revit** + bảng ánh xạ 1:1 sang IFC (D-14) | Người dùng Revit quen ngay, link Revit tự nhiên, đơn giản hơn IFC | Phải duy trì ánh xạ IFC | Revit, ArchiCAD (tương tự) |
| **B. IFC là dữ liệu gốc** (mọi element là thực thể IFC) | Chuẩn mở tuyệt đối, không mất dữ liệu khi xuất IFC | IFC rất cồng kềnh, chậm, không thiết kế cho tương tác nhanh; IFC thay đổi theo phiên bản (IFC4 → IFC5) | Bonsai |
| **C. Tự định nghĩa hoàn toàn** | Tự do | Mất cả hai hệ sinh thái | — |

**Khuyến nghị:** **A**, với yêu cầu: mỗi category, parameter built-in và quan hệ
đều có cột ánh xạ IFC4 (và theo dõi IFC5) ngay trong spec. Thêm trường
`classification` (Uniclass / OmniClass / mã riêng VN) như một parameter chuẩn.
**Độ khó đổi:** 🔴
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-E2. Level (Revit) hay Story (ArchiCAD)? 🟠

**Khác biệt:** Revit: level là mặt phẳng tham chiếu; element bám level bằng offset.
ArchiCAD: story là "tầng" chứa element; mỗi element có "home story", hiển thị
theo tầng, rất tự nhiên với kiến trúc sư.

| Phương án | Được | Mất |
|---|---|---|
| **A. Level kiểu Revit** (D-16) | Link Revit trực tiếp | Không có khái niệm "tầng chứa" rõ ràng |
| **B. Story kiểu ArchiCAD** | Tự nhiên hơn khi thiết kế | Lệch với Revit |
| **C. Level + cờ `is_building_storey`** (level nào là tầng thật, khớp `IfcBuildingStorey`), mỗi element có `level_id` làm "home level"; UI duyệt theo tầng như ArchiCAD | Có cả hai | Cần quy tắc rõ khi element trải nhiều tầng |

**Khuyến nghị:** **C.**
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-E3. Mô hình tường, sàn, mái 🔴

**Vì sao quan trọng:** Đây là phần khó nhất của BIM (nối tường, giao sàn-tường,
lớp cấu tạo bọc góc). Revit mất nhiều năm mới ổn.

**Khuyến nghị (đề xuất tổng thể, cần đồng ý):**
1. **Tường** = `LocationCurve` + `FamilyType` có **cấu tạo nhiều lớp**
   (`CompoundStructure`: lớp, độ dày, chức năng: kết cấu/hoàn thiện/cách nhiệt,
   vật liệu) + `location_line` (tim tường, mặt ngoài lõi...) + base/top level.
   Giống Revit gần như 1:1.
2. **Nối** (join) là quan hệ tường minh trong Document, giải bằng một
   **join engine** riêng (L, T, X, cắt theo ưu tiên chức năng lớp). Spec riêng.
3. **Sàn/mái** = `Profile` trên `WorkPlane` + cấu tạo nhiều lớp + độ dốc.
4. **Form → Wall**: lệnh `convert_form_to_walls` nhận diện khối dạng tấm thành
   tường thật (AI dùng rất nhiều).

| Phương án | Được | Mất |
|---|---|---|
| **A. Như trên, giống Revit** | Ánh xạ Revit hoàn hảo | Khối lượng lớn |
| **B. Tường đơn giản (một lớp) trước, lớp cấu tạo sau** | Nhanh | Phải nâng cấp dữ liệu sau |

**Khuyến nghị:** **A về dữ liệu, B về triển khai**: schema đầy đủ lớp từ đầu, UI và
join engine làm dần.
**TaD trả lời:** ☐ A ☐ B · ghi chú:

---

### Q-E4. Phòng và diện tích 🔴

**Vì sao quan trọng:** Diện tích là thứ chủ đầu tư hỏi đầu tiên. Ở VN có cách tính
riêng (diện tích xây dựng, diện tích sàn, thông thủy, tim tường) — TaD đã làm
plugin cho việc này.

| Phương án | Được | Mất | Ai dùng |
|---|---|---|---|
| **A. Room tự dò biên** từ tường/vách/đường phân chia | Tự cập nhật khi sửa tường | Thuật toán dò biên phải bền | Revit |
| **B. Zone vẽ tay** | Kiểm soát | Không tự cập nhật | ArchiCAD (có cả dò tự động) |
| **C. A + "Area Scheme"** (nhiều cách đo trên cùng model: thông thủy, tim tường, theo luật VN, theo BOMA...) dưới dạng **plugin quy tắc** | Đúng cho từng thị trường | Thêm một lớp khái niệm |

**Khuyến nghị:** **C.** Bộ quy tắc VN (DTXD/DTSD, thông thủy) là plugin chính thức
đầu tiên, là lợi thế cạnh tranh rõ rệt.
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-E5. Hệ parameter: chi tiết 🟠

D-15 đã chốt parameter có kiểu. Câu hỏi còn lại: có làm **Property Set** kiểu IFC
(nhóm parameter theo bộ, vd `Pset_WallCommon.FireRating`) không?

| Phương án | Được | Mất |
|---|---|---|
| **A. Parameter phẳng** (kiểu Revit) + ánh xạ sang Pset khi xuất IFC | Đơn giản | Ánh xạ thủ công |
| **B. Parameter thuộc Group/Pset ngay từ đầu** (key dạng `pset.name`) | Xuất IFC tự nhiên, UI nhóm rõ ràng | Hơi phức tạp hơn |

**Khuyến nghị:** **B** (khóa `ferro.wall.fire_rating` thuộc group `Pset_WallCommon`
qua bảng ánh xạ; parameter tự định nghĩa của người dùng thì chọn group).
**TaD trả lời:** ☐ A ☐ B · ghi chú:

---

### Q-E6. Hệ family (loadable): định nghĩa bằng gì? 🔴

**Vì sao quan trọng:** Cửa, cửa sổ, nội thất, thiết bị... là phần lớn số element.
Revit family editor là nỗi ám ảnh; ArchiCAD GDL là script.

| Phương án | Được | Mất | Ai dùng |
|---|---|---|---|
| **A. Family editor trực quan** (một Document riêng có parameter, ràng buộc, hình học) | Quen thuộc với dân Revit | Khó dùng, khó cho AI | Revit |
| **B. Family bằng script** (khai báo parameter + code sinh hình) | Mạnh, AI viết rất tốt, diff được | Người không code khó dùng | ArchiCAD GDL |
| **C. Family = node graph (Q-D2) + Form có tham số** | Trực quan + tham số | Phụ thuộc node graph | Grasshopper-style |
| **D. Khai báo (dữ liệu) + hình học từ Form + ràng buộc đơn giản**, có thể sinh bởi AI từ ảnh/mô tả; script cho trường hợp nâng cao | AI tạo family từ một tấm ảnh catalog; dễ chỉnh | Nhiều đường cùng lúc |

**Khuyến nghị:** **D**, định dạng family là một `.ferro` nhỏ có `FamilyDefinition`.
Nhập family Revit (`.rfa`) chỉ được qua Revit add-in (định dạng đóng).
**Độ khó đổi:** 🔴
**TaD trả lời:** ☐ A ☐ B ☐ C ☐ D · ghi chú:

---

### Q-E7. Phương án thiết kế (design option) và giai đoạn (phase) 🔴

**Vì sao quan trọng với AI:** AI sinh **nhiều phương án** (3 mặt bằng, 5 kiểu mái).
Nếu không có khái niệm phương án trong lõi, mỗi phương án phải là một file riêng.

| Phương án | Được | Mất |
|---|---|---|
| **A. Design option là khái niệm lõi từ đầu** (mỗi element thuộc "phần chung" hoặc một option trong một option set; so sánh song song, chọn option chính) | AI sinh biến thể ngay trong một model, so sánh, trình bày cạnh nhau | Mọi truy vấn phải biết option đang chọn |
| **B. Dùng nhánh lịch sử** (Q-I3) thay cho option | Một cơ chế cho hai việc | Không xem hai phương án cạnh nhau được |
| **C. Làm sau** | Đơn giản | Sửa lõi về sau rất đau |

**Khuyến nghị:** **A** (khác Revit: option là trung tâm, không phải tính năng phụ).
Phase (hiện trạng/phá dỡ/xây mới) chuẩn bị field từ đầu, làm UI sau — quan trọng
cho cải tạo.
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-E8. Vật liệu 🟠

| Phương án | Được | Mất |
|---|---|---|
| **A. Một element Material gồm**: định danh + thông số vật lý (dẫn nhiệt, khối lượng, chống cháy) + **appearance PBR** (màu, roughness, metallic, texture, normal) + thông tin sản phẩm (hãng, mã, giá) | Một nguồn cho BIM, render và dự toán | Element lớn |
| **B. Tách vật liệu BIM và vật liệu render** | Đơn giản từng phần | Lệch nhau (lỗi kinh điển: Revit render một kiểu, Enscape một kiểu) |

**Khuyến nghị:** **A**, appearance theo chuẩn **glTF PBR / MaterialX** để mang sang
mọi renderer (Q-G5). Cho phép gắn sản phẩm thật (catalog nhà cung cấp VN) → dự
toán và AI gợi ý vật liệu có thật.
**TaD trả lời:** ☐ A ☐ B · ghi chú:

---

### Q-E9. View là element? 🔴

**Vì sao quan trọng:** Camera trình bày, mặt cắt, mặt bằng, style, góc render,
cảnh video đều là "view". Nếu view không phải element, chúng không được lưu, chia
sẻ, AI không điều khiển được.

| Phương án | Được | Mất |
|---|---|---|
| **A. View là element** (camera/vùng cắt/style/bộ lọc hiển thị/option đang xem), có loại: `View3D`, `PlanView` (mặt cắt ngang ở cao độ), `SectionView`, `Elevation`, `Scene` (cho render/video) | AI tạo được "góc nhìn mặt tiền lúc hoàng hôn"; trình bày, video, bản vẽ đều đứng trên đây | Phải thiết kế kỹ ngay |
| **B. View là trạng thái UI** | Đơn giản | Không làm được trình bày, video, cộng tác |

**Khuyến nghị:** **A.** View template (bộ thiết lập hiển thị dùng lại) cũng là element.
**Độ khó đổi:** 🔴
**TaD trả lời:** ☐ A ☐ B · ghi chú:

---

### Q-E10. Site, địa hình, bối cảnh 🟠

**Khuyến nghị:**
- `Site` element: vị trí địa lý (lat/long, độ cao), true north, múi giờ → nắng/bóng
  chính xác.
- `Topography` (lưới tam giác hoặc solid địa hình) + san lấp (pad).
- **Nhập bối cảnh tự động** từ OpenStreetMap (công trình lân cận, đường), bản đồ
  vệ tinh làm nền, cao độ địa hình. Đây là thứ Forma/SketchUp làm và người dùng rất
  thích.
- Ranh đất (property line) với khoảng lùi → AI kiểm tra mật độ, chỉ giới.

**TaD trả lời:** ☐ Đồng ý ☐ Sửa · ghi chú:

---

### Q-E11. Kiểm tra quy chuẩn (rule checking) 🟠

**Vì sao quan trọng:** "AI thiết kế" vô dụng nếu sai quy chuẩn (QCVN 06 PCCC,
QCVN 01 quy hoạch, chiều cao tầng, khoảng lùi, mật độ). TaD đang làm dự án cần
tra PCCC.

| Phương án | Được | Mất |
|---|---|---|
| **A. Engine kiểm tra quy tắc trong lõi** (quy tắc = dữ liệu + hàm kiểm tra, kết quả là `Failure` gắn element) + bộ quy tắc là plugin theo quốc gia | AI tự sửa theo lỗi; người dùng thấy lỗi ngay khi vẽ | Phải chuẩn hóa ngôn ngữ quy tắc |
| **B. Chỉ để AI đọc quy chuẩn rồi đoán** | Nhanh có demo | Không đáng tin, không kiểm chứng được |
| **C. Xuất IFC sang tool kiểm tra ngoài** (Solibri) | Không phải làm | Chậm, không tích hợp |

**Khuyến nghị:** **A + AI đọc văn bản quy chuẩn để soạn quy tắc** (người duyệt),
không để AI phán quyết trực tiếp. Bộ quy tắc VN là plugin chính thức.
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:
