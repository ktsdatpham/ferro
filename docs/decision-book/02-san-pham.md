# A. Sản phẩm, license, quản trị

### Q-A1. Ranh giới sản phẩm: Ferro dừng ở đâu? 🔴

**Vì sao quan trọng:** Quyết định có làm bản vẽ kỹ thuật (mặt bằng có ghi kích
thước, sheet, schedule) hay không sẽ thay đổi toàn bộ kiến trúc: hệ view, hệ chú
thích, in ấn. Đây là phần tốn công nhất của Revit/ArchiCAD.

| Phương án | Được | Mất | Ai đang làm |
|---|---|---|---|
| **A. Thiết kế + trình bày** (concept → schematic → trình bày, render, video). Hồ sơ kỹ thuật làm ở Revit | Tập trung, ra thị trường nhanh, không đối đầu Revit, Revit thành "đồng minh" | Phụ thuộc Revit cho hồ sơ; khách hàng không có Revit phải dùng app khác | Forma, Enscape, SketchUp (thực tế) |
| **B. Thiết kế + trình bày + bản vẽ cơ bản** (mặt bằng/mặt cắt 2D xuất PDF/DWG có kích thước, không làm sheet set đầy đủ) | Đủ cho nhà phố, dự án nhỏ (rất lớn ở VN) tự làm trọn trong Ferro | Phải xây hệ view 2D + annotation sớm hơn | SketchUp + LayOut, Vectorworks |
| **C. Thay thế toàn bộ Revit** | Thị trường lớn nhất | Chục năm công sức, đối đầu trực diện Autodesk | ArchiCAD, Revit |

**Khuyến nghị:** **A trong 2 năm đầu, thiết kế kiến trúc sẵn cho B.** Nghĩa là
hệ view được thiết kế như element từ đầu (Q-E9) nhưng chỉ làm view trình bày;
bản vẽ có kích thước làm khi đã có người dùng. C không bao giờ là mục tiêu
chính thức.
**Độ khó đổi:** 🔴 (ảnh hưởng hệ view và annotation)
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-A2. Người dùng chính là ai? 🔴

| Phương án | Được | Mất |
|---|---|---|
| **A. Kiến trúc sư/designer chuyên nghiệp** giai đoạn concept | Đúng nỗi đau "Revit khó thiết kế", trả tiền được, phản hồi chất lượng | Đòi hỏi chính xác, interop Revit/DWG phải tốt |
| **B. Chủ nhà / người không chuyên** ("mô tả nhà, AI vẽ") | Thị trường rộng, AI tạo wow | Dễ thành đồ chơi, khó kiếm tiền, cạnh tranh với app AI tiêu dùng |
| **C. Sinh viên kiến trúc** | Cộng đồng, lan truyền, mã nguồn mở hợp | Ít tiền, nhưng là người dùng tương lai |
| **D. Developer/BIM coordinator** (dùng như nền tảng để build tool) | Hệ sinh thái plugin | Không ai dùng nếu sản phẩm lõi không tốt |

**Khuyến nghị:** **A là chính, C là kênh lan truyền.** B làm sau như một chế độ
"wizard" trên cùng lõi. Mọi quyết định UX ưu tiên kiến trúc sư đang làm concept.
**Độ khó đổi:** 🟠
**TaD trả lời:** ☐ A ☐ B ☐ C ☐ D · ghi chú:

---

### Q-A3. License mã nguồn 🔴

**Vì sao quan trọng:** Phải chốt **trước khi có người đóng góp**, vì đổi license
sau đó cần sự đồng ý của tất cả người đã đóng góp (hoặc phải có CLA, xem Q-A4).

| Phương án | Được | Mất | Ai dùng |
|---|---|---|---|
| **A. MIT / Apache-2.0** (hiện tại) | Thoải mái nhất, doanh nghiệp dễ dùng, nhiều người đóng góp | **Autodesk hay bất kỳ ai cũng được lấy về đóng thành sản phẩm riêng**, không trả lại gì | Bevy, wgpu, phần lớn hệ sinh thái Rust |
| **B. MPL-2.0** (copyleft theo file) | Ai sửa file của Ferro phải công bố sửa đổi file đó; vẫn được kết hợp với code đóng; plugin đóng được | Một số công ty e ngại; bảo vệ yếu hơn GPL | Firefox |
| **C. GPL-3.0** | Mọi bản phân phối phải mở toàn bộ | Plugin thương mại khó; doanh nghiệp tránh | Blender, FreeCAD, Bonsai |
| **D. AGPL-3.0** | Như GPL và áp dụng cả khi chạy dịch vụ cloud | Doanh nghiệp rất tránh; giảm người đóng góp từ công ty | Grafana, nhiều open-core |
| **E. Mở lõi (A hoặc B) + phần cloud/AI hosted đóng** (open core) | Có mô hình kinh doanh rõ | Phải tách ranh giới rõ, cộng đồng có thể nghi ngờ | GitLab, Supabase |

**Khuyến nghị:** **MPL-2.0 cho lõi + Apache-2.0 cho SDK plugin/schema**, kết hợp
mô hình E (dịch vụ cloud/AI hosted là sản phẩm kinh doanh). MPL bảo vệ lõi khỏi
bị lấy về đóng mà vẫn cho phép plugin thương mại và doanh nghiệp dùng.
**Độ khó đổi:** 🔴
**TaD trả lời:** ☐ A ☐ B ☐ C ☐ D ☐ E · ghi chú:

---

### Q-A4. Bản quyền đóng góp: CLA hay DCO? 🔴

| Phương án | Được | Mất |
|---|---|---|
| **A. CLA** (người đóng góp ký trao quyền cấp license cho dự án/công ty) | Đổi license sau này được; bán license thương mại kép được | Nhiều người ngại ký; cộng đồng nghi ngờ "công ty sẽ đóng code" |
| **B. DCO** (chỉ cần `Signed-off-by` trong commit, xác nhận có quyền đóng góp) | Nhẹ, thân thiện, chuẩn của Linux kernel | Không đổi license được nếu không có sự đồng ý của mọi người |
| **C. Không có gì** | Dễ nhất | Rủi ro pháp lý khi doanh nghiệp dùng |

**Khuyến nghị:** **B (DCO)** nếu chọn MPL và không định bán license kép. **A**
nếu TaD Design muốn giữ quyền bán bản thương mại riêng.
**Độ khó đổi:** 🔴
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-A5. Mô hình kinh doanh

| Phương án | Được | Mất | Ví dụ |
|---|---|---|---|
| **A. App miễn phí, AI/cloud trả phí** (AI hosted, sync, render cloud, lưu trữ) | Phù hợp mã nguồn mở, doanh thu định kỳ | Phải vận hành server | Figma-like, Supabase |
| **B. Bring-your-own-key AI miễn phí + gói Pro** (asset library, render farm, hỗ trợ) | Người dùng kiểm soát chi phí AI | Doanh thu thấp hơn |
| **C. Bán plugin/dịch vụ doanh nghiệp** (Revit link Pro, triển khai on-premise, đào tạo) | Hợp với TaD Design hiện tại | Phụ thuộc dịch vụ |
| **D. Tài trợ / quỹ** | Giữ độc lập | Không bền | Blender Foundation |

**Khuyến nghị:** **A + B + C kết hợp**: người dùng có thể tự cắm API key (miễn phí);
hoặc mua gói Ferro Cloud (AI hosted, sync, render cloud, asset library); doanh
nghiệp mua triển khai on-premise + Revit link + hỗ trợ.
**Độ khó đổi:** 🟢 (nhưng ảnh hưởng Q-A3)
**TaD trả lời:** ☐ A ☐ B ☐ C ☐ D · ghi chú:

---

### Q-A6. Quản trị dự án mã nguồn mở

| Phương án | Được | Mất |
|---|---|---|
| **A. BDFL** (TaD quyết cuối cùng) + maintainer theo mảng | Nhanh, nhất quán (rất quan trọng giai đoạn đầu) | Phụ thuộc một người |
| **B. Hội đồng kỹ thuật** | Bền, dân chủ | Chậm, dễ tranh cãi |
| **C. Foundation** độc lập | Doanh nghiệp tin tưởng | Quá sớm |

**Khuyến nghị:** **A trong giai đoạn 0–2 năm**, chuyển dần sang B khi có ≥5
maintainer tích cực. Mọi quyết định nền móng qua ADR công khai. Mỗi crate có
`CODEOWNERS`.
**Độ khó đổi:** 🟢
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-A7. Chính sách đóng góp bằng AI

**Vì sao quan trọng:** Ferro chủ động mời AI đóng góp. Không có luật thì repo sẽ
ngập PR do AI sinh ra, dài, sai kiến trúc, không ai review nổi.

| Phương án | Được | Mất |
|---|---|---|
| **A. Cho phép, nhưng người gửi PR chịu trách nhiệm hoàn toàn**, PR nhỏ, phải theo AGENTS.md, phải ghi rõ có dùng AI | Tận dụng AI, vẫn kiểm soát | Cần CI mạnh để lọc |
| **B. Cấm** | Đơn giản | Ngược với tầm nhìn |
| **C. Cho phép không giới hạn** | Nhiều code | Rác công nghệ |

**Khuyến nghị:** **A**, thêm: CI kiểm tra kiến trúc (lint phụ thuộc tầng, cấm
từ ngoài glossary), giới hạn kích thước PR, bắt buộc test, nhãn `ai-assisted`.
**Độ khó đổi:** 🟢
**TaD trả lời:** ☐ A ☐ B ☐ C · ghi chú:

---

### Q-A8. Tên và thương hiệu

"Ferro" ngắn, dễ nhớ, nhưng là từ phổ biến (có công ty/sản phẩm trùng tên ở các
ngành khác). Cần tra thương hiệu tại VN (Cục SHTT), US (USPTO), EU (EUIPO) trong
nhóm phần mềm (class 9, 42) và kiểm tra domain trước khi quảng bá.

**Khuyến nghị:** Giữ "Ferro" làm tên dự án, tra cứu trước khi ra bản 0.5; nếu
trùng, đổi trước khi có người dùng (code đã chuẩn bị để đổi tên dễ).
**TaD trả lời:** ☐ Giữ, sẽ tra cứu ☐ Đổi tên · ghi chú:
