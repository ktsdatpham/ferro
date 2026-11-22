# 1. Đối thủ và bài học

> Thông tin thị trường theo hiểu biết đến giữa 2026; giá và tình trạng sản phẩm
> thay đổi nhanh, cần kiểm tra lại trước khi trích dẫn công khai.

## 1.1 Thế hệ CAD cổ điển

| App | Mạnh | Yếu | Bài học cho Ferro |
|---|---|---|---|
| **AutoCAD** (1982) | Chuẩn công nghiệp 2D, DWG ở khắp nơi, command line cực nhanh, LISP/.NET mở rộng | Không có ngữ nghĩa (chỉ là nét), 3D yếu, phần lớn vẫn là kiến trúc cũ | **Command line + alias** là thứ người dùng chuyên nghiệp yêu; DWG là thứ phải đọc được |
| **MicroStation** (Bentley) | Xử lý dự án hạ tầng khổng lồ, reference file (federation) rất tốt | Khó học, UI rối | **Federation**: dự án lớn = nhiều file tham chiếu nhau, không phải một file khổng lồ |
| **BricsCAD** | Tương thích DWG, có BIM, kernel ACIS, giá rẻ | Thị phần nhỏ | Có thể sống bằng cách **tương thích hệ sinh thái** đối thủ |
| **ArchiCAD** (Graphisoft, 1987) | BIM đầu tiên dành cho kiến trúc sư, **thân thiện hơn Revit**, story/zone tốt, Teamwork (cộng tác) từ sớm, GDL family bằng script, chạy Mac tốt | Kết cấu/MEP yếu hơn Revit, thị phần nhỏ hơn ở US/VN | **Thiết kế cho kiến trúc sư trước**; GDL cho thấy family bằng ngôn ngữ script là khả thi; Teamwork chứng minh cộng tác real-time trong BIM |
| **Vectorworks** | 2D + 3D + BIM + landscape + render trong một app, trình bày đẹp | Ôm đồm, không xuất sắc ở mảng nào | Trình bày (presentation) là điểm bán hàng thật với kiến trúc sư |
| **Allplan / Tekla** | Kết cấu, thép, bê tông chi tiết | Ngách | Không cạnh tranh mảng chi tiết thi công; để Revit/Tekla làm |

## 1.2 Thế hệ BIM thống trị

| App | Mạnh | Yếu | Bài học |
|---|---|---|---|
| **Revit** | Mô hình dữ liệu chặt (element, category, type, parameter, transaction), bản vẽ tự cập nhật, đa ngành, API .NET lớn | **Khó thiết kế**, chậm với model lớn, đơn luồng phần lớn, file khổng lồ, worksharing khó, đắt, chỉ Windows, family editor rối | Lấy **mô hình dữ liệu** của Revit, bỏ **trải nghiệm** của Revit. Hiệu năng và đa luồng phải có từ ngày đầu |

## 1.3 Thế hệ modelling tự do

| App | Mạnh | Yếu | Bài học |
|---|---|---|---|
| **SketchUp** | Push-pull + **inference engine** huyền thoại, học trong 1 giờ, 3D Warehouse, Ruby plugin, style trình bày (sketchy, line) | Hình dính nhau (sticky geometry), mesh không có ngữ nghĩa BIM, chậm với model lớn, Ruby API đơn luồng, mặt cong là mesh | **Inference + push-pull là trải nghiệm cốt lõi phải vượt qua.** Style trình bày là lý do kiến trúc sư VN dùng SketchUp. Group mặc định để tránh dính hình |
| **Rhino + Grasshopper** | NURBS chính xác, command line + autocomplete, **Grasshopper** (parametric bằng node), plugin cực phong phú, openNURBS mở | Không phải BIM, cần plugin (VisualARQ, Rhino.Inside.Revit) | **Node graph** cho parametric, openNURBS (đọc .3dm miễn phí), Rhino.Inside chứng minh nhúng vào Revit được |
| **Blender** (+ **Bonsai**, trước là BlenderBIM) | Mã nguồn mở, render Cycles/Eevee, animation, video, Geometry Nodes, cộng đồng khổng lồ; Bonsai dùng **IFC làm dữ liệu gốc** | UI không dành cho kiến trúc sư, chính xác CAD yếu | **Mô hình cộng đồng mã nguồn mở thành công nhất**; GPL; IFC-native khả thi nhưng khó dùng |
| **Plasticity / Shapr3D** | UX hiện đại, Shapr3D chạy iPad cực tốt với bút | Kernel thương mại (Parasolid), không BIM | **iPad + bút** là thị trường thật; UX hiện đại bán được |
| **FreeCAD** | Mở hoàn toàn, có BIM workbench, OpenCascade | **Topological naming problem** gây gãy model hàng chục năm, UI khó | Ví dụ kinh điển của việc **không chốt nền móng sớm** |

## 1.4 Thế hệ cloud / web / AI (2018–2026)

| App | Hướng | Bài học |
|---|---|---|
| **Figma** (không phải AEC) | Web, multiplayer real-time, CRDT-like, plugin sandbox | Chứng minh app thiết kế nặng **chạy được trên web** (WASM + WebGL/WebGPU) và cộng tác real-time là chuẩn mới |
| **Onshape** (CAD cơ khí) | CAD 100% cloud, branch/merge như git | **Lịch sử có nhánh** cho mô hình 3D là khả thi và được yêu thích |
| **Autodesk Forma** (từ Spacemaker) | Thiết kế sơ bộ trên cloud: khối, phân tích nắng/gió/tiếng ồn, AI | Autodesk thừa nhận giai đoạn **concept** đang bị bỏ trống; phân tích môi trường sớm là giá trị thật |
| **Snaptrude, Arcol, Motif, Qonic** | BIM trên trình duyệt, cộng tác, AI, nhắm "Revit cho giai đoạn thiết kế" | Đúng hướng của Ferro: thị trường đã xác nhận nhu cầu. Điểm Ferro khác: **mã nguồn mở + native nhanh + AI qua MCP mở** |
| **TestFit, Higharc, Hypar** | Generative: tự sinh layout căn hộ/bãi xe/nhà ở theo quy tắc | **Generative theo luật** chạy nhanh và đáng tin hơn để AI đoán; AI nên gọi các bộ sinh này như tool |
| **Speckle, That Open Company (IFC.js)** | Hạ tầng dữ liệu AEC mở | Có thể **dùng/kết nối** thay vì tự làm mọi thứ interop |

## 1.5 Render và trình bày

| App | Mạnh | Bài học |
|---|---|---|
| **Enscape** (nay thuộc Chaos) | Real-time một click trong Revit/SketchUp/Rhino, live sync | Người dùng muốn **render ngay trong app thiết kế**, không export |
| **Twinmotion, Lumion, D5** | Real-time đẹp, asset library lớn, video dễ | **Thư viện asset (cây, người, xe, nội thất) + video** là lý do người ta trả tiền |
| **V-Ray, Corona** | Ảnh tĩnh chất lượng cao nhất | Path tracing cho ảnh cuối, real-time cho làm việc |
| **AI render** (Veras, LookX, Stable Diffusion/ControlNet, Midjourney) | Biến khối thô thành ảnh đẹp trong vài giây | **Viewport + AI diffusion** thay thế phần lớn render truyền thống ở giai đoạn ý tưởng; nhưng AI không giữ đúng hình học → cần kiểm soát bằng depth/edge/segmentation |

## 1.6 Bài học tổng hợp: Ferro thắng ở đâu

| Khoảng trống thị trường | Ferro lấp bằng |
|---|---|
| Revit khó thiết kế | Tương tác kiểu SketchUp trên dữ liệu kiểu Revit |
| SketchUp không có dữ liệu | Mọi thứ là element có category + parameter, xuất Revit/IFC |
| App AI hiện tại là hộp đen, đóng | **AI mở qua MCP**, model nào cũng cắm được, mã nguồn mở |
| Render phải chuyển app | Viewport đẹp sẵn + style trình bày + AI render + video trong cùng app |
| Concept → hồ sơ bị đứt | Live link Revit, IFC chuẩn |
| App web chậm, app desktop không cộng tác | Native Rust nhanh + local-first + sync |
| Không có app nào hiểu quy chuẩn VN | Bộ luật QCVN/TCVN, cách tính diện tích VN là plugin hạng nhất |

## 1.7 Những cái chết cần tránh

1. **Ôm đồm quá sớm** (Vectorworks-hóa): làm 10 thứ tầm thường thay vì 1 thứ xuất sắc.
2. **Nền móng hình học yếu** (FreeCAD): tham chiếu gãy, boolean nổ, mất niềm tin người dùng.
3. **Không tương thích** hệ sinh thái DWG/Revit: kiến trúc sư không thể bỏ quy trình hiện tại.
4. **Cloud-only ở thị trường mạng chập chờn / dữ liệu nhạy cảm**: VN và nhiều nước cần offline.
5. **Mã nguồn mở không có người quản trị**: hàng trăm PR không ai duyệt, mỗi người một kiểu.
6. **AI làm demo đẹp nhưng không dùng được thật**: kết quả không chỉnh được, không đúng kích thước, không lặp lại được.
