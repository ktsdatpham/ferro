# B. Lõi dữ liệu (kernel)

Đây là "nhân hệ điều hành" của Ferro. Mọi thứ khác (AI, render, cộng tác, plugin)
đứng trên nó. Sai ở đây thì không vá được, chỉ có viết lại.

### Q-B1. Document được lưu trong bộ nhớ theo cấu trúc nào? 🔴

**Vì sao quan trọng:** Quyết định tốc độ truy vấn (AI hỏi "liệt kê mọi tường tầng
2"), tốc độ undo, khả năng đa luồng, và khả năng cộng tác real-time.

| Phương án | Được | Mất | Ai dùng |
|---|---|---|---|
| **A. Arena + struct có kiểu** (`BTreeMap<ElementId, Element>`, element là enum Rust) | Đơn giản, dễ hiểu cho người đóng góp, compiler bắt lỗi | Undo phải copy hoặc ghi ngược; truy vấn theo thuộc tính chậm nếu không có index | Đa số app CAD cổ điển, Ferro v0.1 |
| **B. ECS** (Entity-Component-System, vd `bevy_ecs`) | Truy vấn cực nhanh theo component, song song tự nhiên, gắn component mở rộng dễ (plugin thêm dữ liệu) | Khái niệm lạ với dân BIM, ràng buộc giữa element khó biểu diễn, undo/serialize phải tự làm | Game engine, một số app CAD mới |
| **C. Cấu trúc bất biến chia sẻ** (persistent data structures, vd `im`/`rpds`): mỗi transaction tạo một "phiên bản" mới chia sẻ phần không đổi | **Undo/redo gần như miễn phí**, snapshot cho đa luồng và render không cần khóa, dễ so sánh phiên bản (diff), hợp với cộng tác | Ghi chậm hơn một chút (hằng số), bộ nhớ cần quản lý | Clojure/Datomic, nhiều editor hiện đại |
| **D. Database nhúng** (SQLite trong bộ nhớ làm nguồn sự thật) | Truy vấn SQL mạnh, lưu file tự nhiên | Chậm cho thao tác tương tác 60fps, không hợp hình học | Bentley iModel (dùng SQLite cho lưu trữ) |

**Khuyến nghị:** **C làm nền + index phụ kiểu A**: Document là cây bất biến chia sẻ
(`ElementId → Arc<Element>`), mỗi commit sinh `DocumentVersion` mới. Render, AI,
regeneration đọc một snapshot cố định trên luồng riêng không cần khóa; undo là
quay về version cũ. Thêm index (theo category, level, không gian/BVH) cập nhật
theo từng commit. ECS chỉ dùng trong renderer.
**Độ khó đổi:** 🔴
**TaD trả lời:** ☐ A ☐ B ☐ C ☐ D · ghi chú:

---

### Q-B2. Schema element: cứng trong code hay động? 🔴

**Vì sao quan trọng:** Plugin và AI muốn thêm loại dữ liệu mới (vd "đánh giá phong
thủy", "hệ số K cách nhiệt theo QCVN"). Nếu schema cứng, mọi thứ mới phải sửa lõi.
Nếu quá động, mất an toàn kiểu và AI dễ tạo rác.

| Phương án | Được | Mất | Ai dùng |
|---|---|---|---|
| **A. Cứng hoàn toàn** (enum Rust, mỗi category một struct) | An toàn, nhanh, dễ tối ưu | Plugin không thêm loại mới được | Revit (built-in) |
| **B. Động hoàn toàn** (mọi thứ là thuộc tính tên/giá trị, schema ở file) | Linh hoạt tối đa | Chậm, dễ sai, mất trợ giúp của compiler | IFC/EXPRESS, Bonsai |
| **C. Lai:** lõi cứng (Wall, Floor, Level, Form...) + **parameter có kiểu mở rộng được** (shared/project parameter, Q-E5) + **component mở rộng có schema đăng ký** cho plugin | Lõi an toàn, mở rộng có kiểm soát; giống Revit (built-in + shared parameter + Extensible Storage) | Phức tạp hơn A một chút | Revit (Extensible Storage), ArchiCAD (property) |

**Khuyến nghị:** **C.** Plugin đăng ký schema (tên, phiên bản, kiểu trường) trước khi
ghi dữ liệu; dữ liệu của plugin không có mặt vẫn được giữ nguyên khi mở file
(không mất dữ liệu của người khác).
**Độ khó đổi:** 🔴
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-B3. Nguồn sự thật: trạng thái hay nhật ký lệnh? 🔴

**Vì sao quan trọng:** AI làm việc bằng chuỗi lệnh. Nếu lưu lại chuỗi lệnh, ta có
thể phát lại, nhánh, giải thích "vì sao tường này ở đây", và cộng tác.

| Phương án | Được | Mất |
|---|---|---|
| **A. Trạng thái là sự thật** (lưu kết quả; lệnh chỉ để undo trong phiên) | Đơn giản, file nhỏ, mở nhanh | Mất lịch sử "vì sao" |
| **B. Event sourcing** (nhật ký lệnh là sự thật, trạng thái tính lại từ lệnh) | Lịch sử hoàn hảo, phát lại, debug | Phát lại chậm với dự án lâu năm; lệnh cũ phải chạy đúng mãi mãi (khóa chặt hành vi) |
| **C. Trạng thái là sự thật + nhật ký lệnh đi kèm** (lưu snapshot định kỳ + lệnh giữa các snapshot; lệnh để audit, cộng tác, giải thích; không bắt buộc phát lại từ đầu) | Có cả hai, không bị khóa hành vi lệnh cũ | Tốn thêm dung lượng (nén được) |

**Khuyến nghị:** **C.** Mỗi lệnh ghi kèm: ai (người/agent nào), lúc nào, prompt nào
sinh ra nó (nếu là AI). Đây là nền cho Q-H7 (truy vết AI) và Q-I1 (cộng tác).
**Độ khó đổi:** 🔴
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-B4. Định dạng file `.fro` 🔴

| Phương án | Được | Mất | Ai dùng |
|---|---|---|---|
| **A. JSON thuần** (hiện tại) | Đọc được bằng mắt, AI đọc trực tiếp, dễ debug | Chậm, nặng với dự án lớn; lưu là ghi lại toàn bộ | Ferro v0.1 |
| **B. Zip chứa JSON + buffer nhị phân** | Nén tốt, hình học nhị phân nhanh | Lưu vẫn phải ghi lại cả file | .3mf, .docx, .skp (dạng khác) |
| **C. SQLite** (một file, bảng element + blob hình học + bảng lịch sử) | **Lưu từng phần** (chỉ ghi phần đổi), chịu lỗi tốt (mất điện không hỏng file), truy vấn được không cần mở app, chứa được lịch sử và asset | Không đọc bằng mắt; diff git khó | Bentley iModel, Lightroom, nhiều app desktop |
| **D. Thư mục văn bản** (mỗi element một file) | Diff bằng git tuyệt vời | Hàng chục nghìn file, chậm | Một số tool code-first |

**Khuyến nghị:** **C làm định dạng chính** + **xuất/nhập JSON** (định dạng trao đổi,
cho AI và git). Schema bảng có `format_version` và migration (Q-B9).
**Độ khó đổi:** 🔴 (đổi được nhờ migration, nhưng tốn)
**TaD trả lời:** ☐ A ☐ B ☐ C ☐ D · ghi chú:

---

### Q-B5. Dự án lớn: một file hay nhiều file liên kết? 🔴

**Vì sao quan trọng:** Đồ án lớn (khu đô thị, bệnh viện, tòa 40 tầng) có hàng trăm
nghìn element và nhiều nhóm. Revit chậm vì nhồi vào một file.

| Phương án | Được | Mất | Ai dùng |
|---|---|---|---|
| **A. Một file khổng lồ** | Đơn giản, mọi tham chiếu nội bộ | Chậm, khó cộng tác, rủi ro hỏng file | Revit (mặc định) |
| **B. Federation**: dự án = nhiều Document liên kết (kiến trúc / kết cấu / từng tòa / site), tham chiếu chéo bằng `Uid` | Mỗi phần mở nhanh, nhóm làm song song, giống cách đội dự án thật chia việc | Tham chiếu chéo file phức tạp hơn | MicroStation, Revit links, Navisworks |
| **C. B + tải theo nhu cầu**: Document lớn chỉ tải element trong vùng nhìn / đang sửa | Mở dự án 1 triệu element trong vài giây | Phức tạp nhất | Game engine streaming, Qonic |

**Khuyến nghị:** **Thiết kế cho B ngay từ đầu** (ID và tham chiếu đã chuẩn bị qua
`Uid`), làm C khi có nhu cầu thật. Thêm khái niệm `Project` (tập hợp nhiều
`Document` liên kết, chung hệ tọa độ shared).
**Độ khó đổi:** 🔴
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-B6. Mô hình đa luồng 🟠

| Phương án | Được | Mất |
|---|---|---|
| **A. Đơn luồng hoàn toàn** | Đơn giản | Model lớn là đơ (lỗi kinh điển của Revit, SketchUp) |
| **B. Một luồng ghi + nhiều luồng đọc snapshot** (nhờ Q-B1-C): UI không bao giờ chờ regeneration; render, AI, xuất file chạy song song | UI luôn mượt, an toàn (không race) | Cần thiết kế "kết quả chưa sẵn sàng" trên UI |
| **C. Ghi song song nhiều luồng** | Nhanh nhất về lý thuyết | Cực khó đúng, không cần thiết |

**Khuyến nghị:** **B.** Regeneration bên trong một transaction được phép chạy
song song (rayon) theo đồ thị phụ thuộc. Không dùng async runtime trong lõi.
**Độ khó đổi:** 🟠
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-B7. Định danh: dạng `Uid` 🟠

D-13 đã đề xuất `ElementId` (u64, nội bộ) + `Uid` (UUID v7). Câu hỏi còn lại:

| Phương án | Được | Mất |
|---|---|---|
| **A. UUID v7** (có thời gian, sắp xếp được) | Chuẩn, thư viện khắp nơi, chuyển sang IFC GlobalId được | 16 byte |
| **B. UUID v4** (ngẫu nhiên) | Chuẩn | Không sắp xếp theo thời gian, index kém hơn |
| **C. ID tự đặt** (vd ULID, hay số tăng dần theo máy) | Ngắn | Không chuẩn, khó với IFC |

**Khuyến nghị:** **A.**
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-B8. Tính tất định (determinism) 🟠

**Vì sao quan trọng:** Cộng tác, phát lại lệnh, test so sánh, và AI đánh giá kết quả
đều cần "cùng đầu vào → cùng kết quả" trên Windows, Mac, iPad.

| Phương án | Được | Mất |
|---|---|---|
| **A. Tất định nghiêm ngặt trên mọi nền tảng** (cấm hàm toán không chuẩn như `sin` của hệ thống, dùng thư viện toán riêng như `libm`, cấm FMA tự động) | Phát lại lệnh cho kết quả giống nhau từng bit | Chậm hơn một chút, kỷ luật cao |
| **B. Tất định trong cùng nền tảng, sai khác trong dung sai giữa các nền tảng** | Dễ làm | Cộng tác phải đồng bộ trạng thái, không chỉ lệnh |

**Khuyến nghị:** **B cho hình học, A cho logic** (ID, thứ tự, đặt tên, kết quả
generative với seed). Cộng tác đồng bộ **kết quả** của lệnh, không tin vào phát
lại trên máy khác.
**Độ khó đổi:** 🟠
**TaD trả lời:** ☐ A ☐ B · ghi chú:

---

### Q-B9. Tiến hóa schema và tương thích file 🟠

| Phương án | Được | Mất |
|---|---|---|
| **A. Migration tuần tự** (`v1→v2→v3`, file cũ luôn mở được bằng app mới) + app cũ mở file mới ở chế độ chỉ đọc nếu được | Người dùng tin tưởng | Phải viết migration mỗi lần đổi |
| **B. Không cam kết trước 1.0** | Nhanh lúc đầu | Mất người dùng sớm |

**Khuyến nghị:** **A từ bản 0.1 công khai**, kèm bộ file mẫu của mọi phiên bản trong
CI để đảm bảo luôn mở được. (Bài học: file Revit không mở được ở bản cũ là nỗi
đau lớn của ngành.)
**TaD trả lời:** ☐ A ☐ B · ghi chú:

---

### Q-B10. Thư viện dùng chung (asset, family, vật liệu) lưu ở đâu? 🟠

| Phương án | Được | Mất |
|---|---|---|
| **A. Nhúng vào mỗi file** | File tự chứa, gửi đi là đủ | Nặng, trùng lặp |
| **B. Thư viện ngoài + tham chiếu** (theo `Uid` + phiên bản) | Nhẹ, cập nhật tập trung | Gửi file thiếu thư viện |
| **C. Tham chiếu + bộ đệm nhúng** (file giữ bản sao những gì đang dùng, đồng bộ với thư viện nếu có) | Có cả hai | Phức tạp hơn |

**Khuyến nghị:** **C** (giống Revit nạp family vào project nhưng có liên kết ngược
về thư viện gốc để cập nhật).
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-B11. Truy vấn model (cho AI, schedule, plugin) 🟠

**Vì sao quan trọng:** AI hỏi model liên tục ("tầng 3 có bao nhiêu m² phòng ngủ?").
Cần một ngôn ngữ truy vấn chung thay vì mỗi tool viết vòng lặp riêng.

| Phương án | Được | Mất |
|---|---|---|
| **A. API Rust thuần** (filter/iterator, kiểu `FilteredElementCollector` của Revit) | Nhanh, an toàn | AI phải gọi qua nhiều tool nhỏ |
| **B. Ngôn ngữ truy vấn riêng** (vd `category = Walls and level = "L2" and ferro.wall.height > 3000`) dùng chung cho AI, schedule, filter view, plugin | Một cú pháp cho mọi nơi, AI viết rất tốt | Phải thiết kế và duy trì ngôn ngữ |
| **C. SQL** (qua SQLite hoặc DataFusion) | Chuẩn, AI viết SQL cực tốt | Ánh xạ quan hệ element/parameter sang bảng phải thiết kế kỹ |

**Khuyến nghị:** **A làm nền + C cho AI và schedule** (một "view SQL" chỉ đọc
của model: bảng `elements`, `parameters`, `levels`...). AI viết SQL tốt hơn mọi
ngôn ngữ tự chế.
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:
