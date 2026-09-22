Status: ready-for-agent

# Build sshx v1 from scratch: exact SSH config management, paired VM sessions, and managed tunnels

## Problem Statement

ผู้ใช้มี OpenSSH config จำนวนมาก แยกเป็นไฟล์ส่วนตัว ไฟล์งาน และไฟล์ตาม project ผ่าน `Include` ผู้ใช้ต้องจำ alias จำนวนมาก เปิดหลาย terminal tabs เพื่อสร้าง gateway transit ก่อนเข้า VM และจัดการ password กับ service forwarding เอง การใช้ alias เพียงอย่างเดียวไม่ปลอดภัยเมื่อคนละ Host entries ใช้ alias ซ้ำ และการส่ง alias กลับไปให้ combined root อาจทำให้ OpenSSH เลือกคนละ entry จากที่ผู้ใช้เลือก

บาง VM ใช้เส้นทาง local ไป gateway entry แล้วไป VM entry โดย gateway และ VM อาจใช้ password คนละตัว พอร์ต transit สำหรับเข้า SSH ของ VM เป็น dependency ที่ต้องเปิดอัตโนมัติ ส่วน service forwards เช่น PostgreSQL หรือ Redis ต้องเปิดเฉพาะเมื่อผู้ใช้ขอ ผู้ใช้ยังต้องการ standalone tunnel ที่อยู่ต่อหลัง terminal ผู้สั่งปิด และต้อง query หรือ stop ภายหลังได้โดยไม่หา PID เอง

SSH config บน filesystem ต้องเป็น source of truth ต่อไป ผู้ใช้ต้องแก้ไฟล์ด้วย nvim ได้ แอปจึงต้องค้นหา config ปัจจุบันทุกครั้ง รักษา comments, directive order และ formatting เมื่อแก้ไข และตรวจ broken references หลัง external edits โดยไม่สร้าง host database อีกชุดหรือซ่อมไฟล์เงียบ ๆ

ระบบเดิม `seenark/sshx-rust` ใช้งานไม่ได้และถูกยกเลิกสำหรับงานนี้ `sshx-rs` ต้องเริ่มใหม่จาก greenfield Rust project และเผยแพร่ GitHub release assets ที่ติดตั้งเป็น `sshx` ผ่าน `mise use -g github:seenark/sshx-rs` ได้บน macOS และ Linux

## Solution

สร้าง Rust CLI ชื่อ `sshx` โดยใช้ system OpenSSH เป็น connection engine และมี reusable core แยกจาก thin CLI ใน package เดียว Core จะอ่าน Include graph แบบรักษา provenance, ระบุ HostEntry ด้วย source file และ byte span, รองรับ persistent UUID สำหรับ Pair, แก้ config ด้วย byte-level patches และสร้าง runtime SSH config ที่มีเฉพาะ exact selected Host block

การเชื่อมต่อจะใช้ OpenSSH multiplex masters ที่แอปเป็นเจ้าของ Gateway master เปิด temporary loopback transit port และ VM master เชื่อมผ่าน port นั้นด้วย authentication และ stable host-key identity ของ VM เอง Shell เปิดผ่าน VM master ใน terminal เดิม Session-bound masters ไม่ detach และถูก cleanup เมื่อ session จบ ส่วน standalone masters ใช้ OpenSSH background mode, private control sockets และ locked registry โดยไม่ใช้ PID เป็น ownership identity

Password เดิมจาก `##PASSWORD` ยังใช้ได้ Password ใหม่บันทึกกลับ source config ได้เฉพาะหลัง authenticate สำเร็จและผู้ใช้ยืนยัน Secret ส่งให้ sshpass ผ่าน anonymous pipe และ inherited file descriptor เท่านั้น ไม่ส่งผ่าน argv, environment, runtime config, output, logs หรือ error models

Host-key trust ทำเป็นขั้นตอนแยกก่อน authentication First use แสดง fingerprint ผ่าน OpenSSH และรับ confirmation เฉพาะ interactive mode Actual authentication ใช้ strict checking เสมอ Changed key หยุดโดยไม่ลบหรือแทนที่ key อัตโนมัติ Paired VM ใช้ `HostKeyAlias` ที่ derive จาก immutable VM UUID จึงไม่เปลี่ยนเมื่อ temporary transit port เปลี่ยน

Management commands มี human, JSON และ YAML output จาก redacted data model เดียวกัน Non-interactive mode ไม่เปิด picker, confirmation หรือ password prompt และคืน structured error เมื่อข้อมูลไม่ครบ กำกวม ต้อง trust host key หรือมี port conflict

## User Stories

1. ในฐานะผู้ใช้ macOS ฉันต้องการติดตั้ง `sshx` ผ่าน `mise use -g github:seenark/sshx-rs` เพื่อให้ binary อยู่ใน PATH โดยไม่ build source เอง
2. ในฐานะผู้ใช้ Linux ฉันต้องการติดตั้ง release เดียวกันผ่าน mise เพื่อให้ workflow การติดตั้งเหมือน macOS
3. ในฐานะ maintainer ฉันต้องการ release assets แยกตาม OS และ architecture เพื่อให้ mise เลือก binary ที่ถูกต้องอัตโนมัติ
4. ในฐานะ maintainer ฉันต้องการ pin Rust toolchain เป็น 1.95.0 เพื่อให้ local และ CI builds reproducible
5. ในฐานะผู้ใช้ ฉันต้องการ register combined SSH config root ที่มีอยู่ เพื่อเริ่มใช้โดยไม่ย้ายไฟล์
6. ในฐานะผู้ใช้ ฉันต้องการ register work config root ที่มีอยู่ เพื่อแยก scope งานจากส่วนตัวโดยไม่ merge files
7. ในฐานะผู้ใช้ environment ปัจจุบัน ฉันต้องการให้ setup ตรวจพบ `~/.private-key/private-key/config` เพื่อไม่สร้าง path `~/.private-key/config` ที่ไม่มีจริง
8. ในฐานะผู้ใช้ ฉันต้องการให้ Include traversal ใช้ semantics ของ OpenSSH เพื่อให้ host inventory ตรงกับ config จริง
9. ในฐานะผู้ใช้ ฉันต้องการให้ไฟล์เดียวที่ถูก Include หลายเส้นทางแสดงเป็น entry เดียว เพื่อไม่เห็นรายการซ้ำจาก traversal
10. ในฐานะผู้ใช้ ฉันต้องการให้ entry เดียวเก็บ provenance ของทุก scope และ project ที่เข้าถึงมัน เพื่อให้ filters ยังทำงานเมื่อไฟล์ถูกพบหลายเส้นทาง
11. ในฐานะผู้ใช้ ฉันต้องการ filter hosts ตาม personal/work scope เพื่อหาเครื่องที่เกี่ยวข้องเร็วขึ้น
12. ในฐานะผู้ใช้ ฉันต้องการ filter hosts ตาม project หรือ source file เพื่อแยกเครื่องที่ alias คล้ายกัน
13. ในฐานะผู้ใช้ ฉันต้องการเห็น alias, scope, project, source และ destination โดยไม่เห็น password เพื่อแยก entries ได้อย่างปลอดภัย
14. ในฐานะผู้ใช้ interactive CLI ฉันต้องการพิมพ์ค้นหาและเลือก HostEntry เพื่อไม่ต้องจำ alias ทั้งหมด
15. ในฐานะ automation client ฉันต้องการให้คำสั่งที่ไม่มี host ใน non-interactive mode คืน `HOST_REQUIRED` เพื่อไม่ค้างรอ picker
16. ในฐานะผู้ใช้ ฉันต้องการให้คนละ Host entries ที่ใช้ alias เดียวกันยังอยู่เป็นคนละรายการ เพื่อไม่สูญเสีย config ที่ตั้งใจ copy แยก
17. ในฐานะผู้ใช้ interactive CLI ฉันต้องการเลือก source ที่ถูกต้องเมื่อ alias กำกวม เพื่อเชื่อมต่อ entry ที่เลือกจริง
18. ในฐานะ automation client ฉันต้องการให้ ambiguous alias คืน candidates และ error เพื่อไม่เลือก entry แรกเงียบ ๆ
19. ในฐานะ automation client ฉันต้องการเลือก exact entry ด้วย persistent ID หรือ source และ Host line เพื่อให้คำสั่ง deterministic
20. ในฐานะผู้ใช้ ฉันต้องการสร้าง HostEntry ใน scope, project, folder และ file ที่เลือก เพื่อรักษาโครงสร้าง config เดิม
21. ในฐานะผู้ใช้ ฉันต้องการ preview target file และ patch ก่อนเขียน เพื่อเห็นผลกระทบล่วงหน้า
22. ในฐานะผู้ใช้ ฉันต้องการ preview ที่ redact password เพื่อไม่ให้ secret ปรากฏใน terminal history
23. ในฐานะผู้ใช้ ฉันต้องการให้แอปเพิ่ม Include เฉพาะเมื่อ target file ยังไม่ถูกครอบคลุม เพื่อไม่สร้าง Include ซ้ำ
24. ในฐานะผู้ใช้ ฉันต้องการให้ existing wildcard Include ที่จะครอบไฟล์ใหม่ถูกตรวจพบ เพื่อไม่เพิ่ม direct Include โดยไม่จำเป็น
25. ในฐานะผู้ใช้ ฉันต้องการ update HostEntry โดยรักษา comments, directive order, whitespace และ line endings ของส่วนที่ไม่เกี่ยวข้อง
26. ในฐานะผู้ใช้ ฉันต้องการ delete เฉพาะ selected HostEntry เพื่อไม่ลบ block อื่นที่ alias หรือ destination เหมือนกัน
27. ในฐานะผู้ใช้ ฉันต้องการให้ delete ถูก block เมื่อมี Pair reference เพื่อไม่สร้าง dangling relationship
28. ในฐานะผู้ใช้ ฉันต้องการให้ delete ถูก block เมื่อ managed session หรือ tunnel ยังใช้ entry เพื่อไม่ตัด connection ที่กำลังทำงาน
29. ในฐานะผู้ใช้ ฉันต้องการให้ rename รักษา persistent UUID เพื่อให้ Pair และ VM host-key identity ไม่เปลี่ยน
30. ในฐานะผู้ใช้ nvim ฉันต้องการแก้ config ภายนอกได้ตามเดิม เพื่อไม่ถูกผูกกับ editor ของแอป
31. ในฐานะผู้ใช้ ฉันต้องการให้คำสั่งถัดไปอ่าน config ปัจจุบันและรายงาน broken references เพื่อไม่ใช้ cached host database ที่ล้าสมัย
32. ในฐานะผู้ใช้ ฉันต้องการให้ concurrent external edit ทำให้ managed mutation abort เพื่อไม่ทับการแก้ของฉัน
33. ในฐานะผู้ใช้ ฉันต้องการตั้ง Pair ระหว่าง exact gateway entry และ exact VM entry เพื่อให้ alias ซ้ำไม่ทำให้ route ผิด
34. ในฐานะผู้ใช้ ฉันต้องการให้ Pair setup infer transit destination เฉพาะเมื่อ LocalForward และ VM port ให้ candidate เดียว เพื่อไม่เดา route
35. ในฐานะผู้ใช้ ฉันต้องการระบุ transit host และ port เมื่อ infer ไม่ได้ เพื่อบันทึกความสัมพันธ์ครั้งเดียว
36. ในฐานะผู้ใช้ ฉันต้องการให้ Pair setup assign immutable unique UUID ให้ทั้ง gateway และ VM เพื่อรองรับ references และ host-key identity
37. ในฐานะผู้ใช้ที่ copy Host block ฉันต้องการให้ duplicate UUID ถูกแจ้งเป็น error เพื่อไม่รวมคนละ entry เป็น identity เดียว
38. ในฐานะผู้ใช้ ฉันต้องการเชื่อม direct HostEntry โดยไม่สร้าง gateway transit ที่ไม่จำเป็น
39. ในฐานะผู้ใช้ private-key authentication ฉันต้องการใช้ IdentityFile, ssh-agent และ OpenSSH options เดิม เพื่อไม่ถูกบังคับใช้ password
40. ในฐานะผู้ใช้ password authentication ฉันต้องการให้ password ไม่ปรากฏใน process arguments เพื่อไม่รั่วผ่าน process inspection
41. ในฐานะผู้ใช้ ฉันต้องการให้ gateway และ VM ใช้ password คนละตัวได้ เพื่อเข้า VM ด้วยคำสั่งเดียว
42. ในฐานะผู้ใช้ interactive CLI ฉันต้องการ prompt password แบบซ่อนและระบุ role ว่า gateway หรือ VM เพื่อไม่กรอกผิดเครื่อง
43. ในฐานะผู้ใช้ ฉันต้องการให้ configured password ที่ผิดไม่ถูก retry ซ้ำอัตโนมัติ เพื่อไม่เพิ่ม failed login attempts โดยไม่ตั้งใจ
44. ในฐานะผู้ใช้ ฉันต้องการบันทึก password ใหม่เฉพาะหลัง authenticate สำเร็จและฉันยืนยัน เพื่อไม่เก็บค่าที่ยังพิสูจน์ไม่ได้
45. ในฐานะ automation client ฉันต้องการส่ง role-specific password ผ่าน file descriptor เพื่อไม่ใช้ argv หรือ environment
46. ในฐานะ automation client ฉันต้องการให้ missing หรือ wrong password จบด้วย structured error และไม่ prompt เพื่อไม่ทำให้ job ค้าง
47. ในฐานะผู้ใช้ที่พบ server ครั้งแรก ฉันต้องการเห็น fingerprint และยืนยันด้วยตัวเอง เพื่อสร้าง trust อย่างชัดเจน
48. ในฐานะ automation client ฉันต้องการให้ unknown host key จบด้วย `HOST_KEY_TRUST_REQUIRED` เพื่อไม่ accept key อัตโนมัติ
49. ในฐานะผู้ใช้ ฉันต้องการให้ changed host key หยุด connection เพื่อไม่ข้ามการป้องกัน MITM
50. ในฐานะผู้ใช้ work scope ฉันต้องการ known_hosts แยกจาก personal scope เพื่อไม่ปะปน trust stores
51. ในฐานะผู้ใช้ paired VM ฉันต้องการ stable HostKeyAlias จาก VM UUID เพื่อให้ temporary transit port ใหม่ไม่ทำให้ VM ดูเป็น server ใหม่
52. ในฐานะผู้ใช้ ฉันต้องการให้ gateway master เปิด temporary loopback transit port อัตโนมัติ เพื่อไม่ต้องจำ static port
53. ในฐานะผู้ใช้ ฉันต้องการให้ VM runtime ใช้ temporary port เดียวกับ gateway forward เพื่อให้สอง hops เชื่อมกันถูกต้อง
54. ในฐานะผู้ใช้ ฉันต้องการให้ source `LocalForward`, `HostName` และ `Port` ไม่ถูกเขียนทับระหว่าง connection เพื่อให้ config default คงเดิม
55. ในฐานะผู้ใช้ ฉันต้องการให้สอง sessions ของ Pair เดียวกันใช้ transit ports และ control sockets คนละชุด เพื่อไม่ชนกัน
56. ในฐานะผู้ใช้ ฉันต้องการปิด session หนึ่งโดยไม่ปิด session อื่น เพื่อรักษา ownership boundary
57. ในฐานะผู้ใช้ ฉันต้องการเห็น VM shell ใน terminal เดิม เพื่อไม่เปิด terminal tab ที่สอง
58. ในฐานะผู้ใช้ ฉันต้องการให้ session cleanup VM master ก่อน gateway master เพื่อปิด dependency ตามลำดับ
59. ในฐานะผู้ใช้ ฉันต้องการให้ SIGINT, SIGHUP และ SIGTERM trigger owned-session cleanup เพื่อไม่ทิ้ง masters โดยไม่จำเป็น
60. ในฐานะผู้ใช้ ฉันต้องการให้ error ระบุ stage เช่น gateway trust, gateway auth, transit bind, VM trust, VM auth หรือ service bind เพื่อแก้ปัญหาถูกจุด
61. ในฐานะผู้ใช้ ฉันต้องการให้ local listener readiness ไม่ถูกเรียกว่า VM connection success เพื่อไม่รับรายงานเกินหลักฐาน
62. ในฐานะผู้ใช้ ฉันต้องการเข้า shell ปกติโดยไม่ถูกถามเรื่อง service forwarding เพื่อให้ common path เร็ว
63. ในฐานะผู้ใช้ ฉันต้องการเลือกหลาย service ports จาก repeated `##PORT` เพื่อเปิด DB และ service อื่นพร้อมกัน
64. ในฐานะผู้ใช้ ฉันต้องการกำหนด destination host และ persistent local default ที่ละเอียดกว่า `##PORT` เพื่อรองรับ service topology จริง
65. ในฐานะผู้ใช้ ฉันต้องการ override local service port ต่อ invocation เพื่อแก้ local conflict โดยไม่เปลี่ยน config
66. ในฐานะผู้ใช้ ฉันต้องการ default bind address เป็น `127.0.0.1` เพื่อไม่เปิด service ให้เครื่องอื่นใน network
67. ในฐานะผู้ใช้ interactive CLI ฉันต้องการแก้ local port แล้ว retry เมื่อ port busy เพื่อไม่ต้องเริ่ม workflow ใหม่
68. ในฐานะ automation client ฉันต้องการให้ busy port คืน structured error โดยไม่เลือก port ใหม่เอง เพื่อรักษา deterministic behavior
69. ในฐานะผู้ใช้ ฉันต้องการให้ multi-port request เปิดครบหรือ rollback ทั้งชุดใหม่ เพื่อไม่เข้าใจผิดว่าเปิดครบแล้ว
70. ในฐานะผู้ใช้ ฉันต้องการให้ rollback ไม่ปิด existing sessions, tunnels หรือ external processes เพื่อรักษา ownership
71. ในฐานะผู้ใช้ ฉันต้องการ start standalone tunnel แล้วได้ terminal คืน เพื่อใช้ local DB client ต่อ
72. ในฐานะผู้ใช้ ฉันต้องการปิด launcher terminal แล้ว tunnel ยังทำงาน เพื่อไม่ต้องเก็บ terminal tab ไว้
73. ในฐานะผู้ใช้ ฉันต้องการ list และ status standalone tunnels จาก CLI process ใหม่ เพื่อไม่พึ่ง in-memory state ของ launcher
74. ในฐานะผู้ใช้ ฉันต้องการ stop standalone tunnel ด้วย tunnel ID เพื่อไม่หา PID เอง
75. ในฐานะผู้ใช้ paired tunnel ฉันต้องการให้ standalone request จัดการ gateway transit dependency ด้วย เพื่อไม่เปิดอีก tab
76. ในฐานะผู้ใช้ ฉันต้องการให้ exact active standalone request เดิมคืน ID เดิม เพื่อไม่สร้าง process ซ้ำ
77. ในฐานะผู้ใช้ที่มี copied entries ฉันต้องการให้คนละ source identities ไม่ reuse tunnel ข้ามกัน เพื่อรักษา entry semantics
78. ในฐานะผู้ใช้ ฉันต้องการให้ local port conflict กับ tunnel เดิมคืน error เพื่อไม่ปิดหรือแก้ tunnel เดิมเอง
79. ในฐานะผู้ใช้ ฉันต้องการให้ dead master แสดง status ตามจริง เพื่อไม่คิดว่า tunnel ยังใช้ได้
80. ในฐานะผู้ใช้ ฉันต้องการ explicit restart ด้วย request เดิม เพื่อเปิดใหม่ได้โดยไม่มี automatic reconnect
81. ในฐานะผู้ใช้ ฉันต้องการให้ status แยก master, listener และ application health semantics เพื่อไม่ตีความ transport เป็น service health
82. ในฐานะ automation client ฉันต้องการ JSON และ YAML ที่มี semantics เดียวกัน เพื่อเลือก representation ตาม tooling
83. ในฐานะ automation client ฉันต้องการ exactly one machine document บน stdout เพื่อ parse ได้โดยไม่มี progress text ปะปน
84. ในฐานะ automation client ฉันต้องการ symbolic error code และ stage เพื่อ branch logic โดยไม่ parse human wording
85. ในฐานะผู้ใช้ shell ฉันต้องการให้ remote shell output ยังเป็น terminal stream ปกติ เพื่อไม่ถูกห่อเป็น JSON/YAML
86. ในฐานะผู้ใช้ ฉันต้องการ `doctor` ตรวจ OpenSSH, sshpass, config roots, permissions, host-key store, references, ports และ runtime state เพื่อแยกปัญหาได้เร็ว
87. ในฐานะผู้ใช้ key-only inventory ฉันต้องการให้ setup และ doctor ไม่บังคับ sshpass เพื่อใช้ระบบได้โดยไม่ติด dependency ที่ไม่จำเป็น
88. ในฐานะผู้ใช้ macOS ฉันต้องการ setup docs ใช้ current Homebrew `sshpass` formula เพื่อไม่ใช้ unmaintained tap
89. ในฐานะผู้ใช้ Linux ฉันต้องการ setup docs ที่ผ่านการตรวจบน Linux CI เพื่อไม่อ้าง command ที่ไม่ได้พิสูจน์
90. ในฐานะ maintainer ฉันต้องการ checksums สำหรับทุก release asset เพื่อให้ผู้ใช้ตรวจ artifact integrity ได้
91. ในฐานะ maintainer ฉันต้องการ CI บน macOS และ Linux เพื่อป้องกัน Unix behavior ที่ต่างกัน
92. ในฐานะ maintainer ฉันต้องการให้ release ไม่ถือว่าสำเร็จจน mise ติดตั้งจาก fresh environment ได้ เพื่อพิสูจน์ distribution path จริง
93. ในฐานะ maintainer ฉันต้องการแยก fixture/local OpenSSH results จาก user-server results เพื่อไม่รายงาน acceptance เกินหลักฐาน
94. ในฐานะ maintainer ฉันต้องการ reusable core ที่ไม่มี CLI prompting หรือ output formatting ผูกอยู่ เพื่อรองรับ UI อื่นภายหลังโดยไม่สร้าง UI นั้นใน v1

## Implementation Decisions

- Project เริ่มใหม่จาก greenfield repository `seenark/sshx-rs` ไม่ clone, import หรือ reuse history/source จาก `seenark/sshx-rust`
- Binary และ package ใช้ชื่อ `sshx` Rust toolchain pin ที่ 1.95.0
- ใช้ package เดียวที่ expose reusable library core และ thin CLI binary ไม่สร้าง workspace หลาย crate
- ใช้ system OpenSSH เป็น connection engine Core รับผิดชอบ discovery, exact selection, metadata, runtime config compilation, authentication orchestration และ lifecycle
- SSH config บน filesystem เป็น source of truth ไม่มี host database App settings เก็บเฉพาะ registered roots และ runtime state เก็บเฉพาะ managed process metadata
- Combined root default เป็น `~/.ssh/config` Work root มาจาก setup หรือ invocation override Environment ปัจจุบันใช้ `~/.private-key/private-key/config`
- Discovery traverses Include ณ ตำแหน่งที่พบและเรียง wildcard matches แบบ lexical ตาม OpenSSH Relative user includes resolve จาก `~/.ssh`
- Physical files deduplicate ด้วย canonical identity แต่ catalog เก็บ provenance ทุก traversal path เพื่อรองรับ scope/project filters
- `HostEntry` หมายถึง exact Host block Internal `EntryRef` ใช้ physical file identity และ byte span Alias ไม่ unique
- Existing unmanaged direct entries ไม่ต้องมี UUID แต่ app-created entries และ entries ที่เข้าร่วม Pair ต้องมี immutable unique UUID
- Pair เป็น 1:1 VM metadata อ้าง gateway UUID และบันทึก transit destination Duplicate, malformed หรือ missing UUID ใน Pair เป็น validation error
- Metadata เดิม `##PASSWORD` และ repeated `##PORT` ต้องอ่านได้ทันที Metadata ใหม่ใช้ namespace `##SSHX` สำหรับ ID, GATEWAY, TRANSIT และ detailed SERVICE defaults
- Existing `##PORT` หมายถึง service บน `127.0.0.1` โดย remote และ default local port เท่ากัน Detailed SERVICE metadata สามารถกำหนด destination host และ default local port ของ declared remote port
- Runtime config ใช้ exact selected block และ original alias ตัด comments และ app metadata ทั้งหมดออกเพื่อไม่ copy password เข้า runtime files
- Runtime config explicitly includes system SSH config หลัง selected options เพราะ `-F` ข้าม system config ตามปกติ
- User config constructs ที่ทำให้ exact-block semantics รักษาไม่ได้ เช่น wildcard Host, Match, unsupported global directives, conditional Include หรือ token-sensitive semantics ต้อง fail closed ก่อน `ssh -G`
- Existing trusted direct-host directives เช่น IdentityFile, agent options และ ProxyCommand ถูกส่งต่อเมื่อ semantics รักษาได้ Pair entries ปฏิเสธ ProxyCommand/ProxyJump เพราะขัดกับ explicit two-hop route
- ไม่มี generic arbitrary-option writer สำหรับ CRUD Managed values ใช้ narrow grammar และ reject NUL, CR, LF ไม่มี shell invocation
- Gateway runtime rewritesเฉพาะ chosen transit LocalForward bind เป็น temporary `127.0.0.1` port และคง unrelated forwardings
- VM runtime override HostName เป็น loopback และ Port เป็น temporary transit port เดียวกัน
- Paired route ใช้ gateway และ VM OpenSSH connections แยกกัน ไม่ใช้ ProxyJump เพราะ credentials และ host-key identities แยกกัน
- Direct และ paired interactive sessions ใช้ non-detached OpenSSH masters แล้วเปิด shell ผ่าน control socket เพื่อสังเกต authentication success และ cleanup ownership ได้
- Standalone tunnels ใช้ OpenSSH background master, private control sockets และ `-O` operations ไม่มี custom daemon และไม่ kill PID
- Private control directories เป็น random, user-owned, mode 0700 และ reject symlinks/unexpected file types Control operations ใช้ `-F none` เพื่อไม่ re-evaluate changed user config
- Registry มี lock, atomic writes และ states `starting`, `active`, `stopping`, `failed`, `stale` Request signature รวม exact source identities, block fingerprints, route และ effective forwards แต่ไม่รวม secrets
- Session และ standalone dependencies ไม่ share ข้าม request โดยอัตโนมัติ Exact active standalone request เท่านั้นที่ reuse ID เดิม
- `ExitOnForwardFailure=yes` ใช้พิสูจน์ forward setup เท่านั้น Status ใช้คำว่า master/listener active และไม่อ้าง application health
- App-selected temporary transit port อาจ retry เมื่อเกิด race ส่วน user-selected service port ไม่เปลี่ยนเอง
- Host-key enrollment แยกจาก password authentication Interactive enrollment แสดง OpenSSH fingerprint และต้องรับ explicit confirmation Non-interactive mode fail unknown keys
- Actual authentication fix security-sensitive options รวม strict checking, stable HostKeyAlias, intended known_hosts file, disabled localhost exemption และ disabled dynamic known-host command
- Work known_hosts แยกจาก personal known_hosts VM HostKeyAlias derive จาก immutable VM UUID ไม่ derive จาก alias หรือ temporary port
- Password source บน disk ที่อนุญาตมีเพียง existing/consented `##PASSWORD` ใน source config Password ไม่อยู่ใน public model, runtime config, registry, argv, environment, logs, preview หรือ error
- sshpass รับ password ผ่าน anonymous pipe และ one inherited FD ที่ expose เฉพาะ child นั้น Pipe descriptors ใช้ close-on-exec by default และ writer ปิดทันทีหลัง bounded write
- Config editor ใช้ byte-span patches รักษา unrelated bytes, comments, directive order, modes และ line endings
- Mutations lock writer, verify file identity and digest, write same-directory exclusive temporary file, fsync, atomic rename และ fsync directory Concurrent external changes abort แทน overwrite
- Multi-file operations precompute outputs และเรียง commit ให้ failure ปลอดภัย Create เขียน target ก่อน Include Pair เขียน gateway identity ก่อน VM reference Journal ไม่มี secret
- Delete ไม่ cascade และไม่ stop process เอง Active or referenced entry ทำให้ delete fail พร้อม remediation
- CLI surface ประกอบด้วย default/connect, setup, doctor, host CRUD, pair setup และ tunnel lifecycle commands
- Exact selection ใช้ alias เดี่ยว, UUID เดี่ยว หรือ alias พร้อม source และ Host line Partial/mismatched selectors เป็น error
- `--no-input` ห้าม picker, prompt และ confirmation `--yes` อนุมัติเฉพาะ config mutations ไม่อนุมัติ host keys
- Service forwarding ไม่ prompt ใน normal connect path Interactive selection ต้องขอด้วย `--bind` Machine selection ใช้ repeated `--forward REMOTE[=LOCAL]`
- Human, JSON และ YAML management outputs derive จาก explicit redacted DTOs เดียวกัน Machine stdout มี exactly one versioned envelope
- Shell commands ไม่รองรับ machine wrapping เพราะ stdout/stderr เป็น remote terminal streams
- Error model ใช้ stable symbolic code และ stage อย่างน้อย selection, config, gateway trust/auth, transit bind, VM trust/auth, service bind, session, cleanup และ registry
- Release tags ใช้ `vX.Y.Z` และแนบ macOS/Linux arm64/x86_64 archives ที่มี executable `sshx` ณ archive root พร้อม SHA-256 checksums
- Release acceptance ต้องทดสอบ `mise use -g github:seenark/sshx-rs` ใน fresh mise environment และเรียก `sshx --version` สำเร็จ

## Testing Decisions

- Primary test seam คือ compiled `sshx` CLI process ใช้ temporary HOME, fixture SSH config tree, controlled PATH และ observable filesystem/process output นี่เป็น seam สูงสุดที่พิสูจน์ discovery, selection, mutations, machine contracts และ lifecycle พร้อมกัน
- ไม่มี existing implementation tests ให้ reuse เพราะ project เป็น greenfield Prior art มีเพียง approved acceptance scenarios และ OpenSSH behavior ที่ตรวจจาก system tools
- Tests assert external behavior, exit status, stdout/stderr, resulting config bytes, control-socket behavior และ reachable listeners ไม่ assert private fields, helper calls หรือ command-builder internals
- Config discovery tests ครอบ Include order, relative paths, wildcard lexical order, cycles, physical-file deduplication, provenance, scopes, projects และ duplicate aliases
- Exact-selection tests ใช้ alias ซ้ำที่ resolve ไปคนละ local SSH endpoints เพื่อพิสูจน์ว่า selected source เชื่อมถูก entry ไม่ใช่เพียง list ถูก
- Config mutation tests เรียก CLI แล้วเปรียบเทียบ unrelated bytes แบบ byte-for-byte ครอบ comments, blank lines, CRLF/LF, trailing newline, permissions และ external-edit conflict
- Multi-file mutation tests inject failure ระหว่าง target/Include และ gateway/VM writes แล้ว assert safe recoverable state ไม่มี silent partial success
- Public-output tests parse JSON และ YAML กลับเป็น semantic model เดียวกัน และ scan stdout/stderr/errors/previews เพื่อยืนยันว่าไม่มี password
- Password transport tests ใช้ fake TTY password program ผ่าน actual sshpass เพื่อพิสูจน์ FD transport, separate gateway/VM secrets, bad-password exit และ absence from argv/output
- OpenSSH integration harness รัน temporary local sshd instances บน high portsเพื่อพิสูจน์ private-key auth, control masters, forwarding, host-key first use, changed key และ stable HostKeyAlias
- Paired-route tests ใช้ gateway และ VM endpoints แยกกัน เริ่ม gateway ก่อน VM และ assertว่า VM authentication เป็น end-to-end route proof
- Concurrent-session tests เปิด Pair เดียวกันสอง sessions แล้ว assert temporary ports/control directories ต่างกันและ cleanup หนึ่ง sessionไม่กระทบอีก session
- Service-forward tests bind one requested local port ด้วย unrelated process แล้ว assert no partial new set remains and unrelated process stays alive
- Standalone tests start tunnel จาก process หนึ่ง รอ process จบ แล้ว query/stop จาก process ใหม่เพื่อพิสูจน์ persistence ข้าม terminal
- Registry concurrency tests start identical requests พร้อมกันแล้ว assert one active tunnel ID and deterministic starting/active outcome
- Dead-state tests terminate an owned master through its control channel แล้ว assert status reports down and does not reconnect
- Signal testsส่ง SIGINT, SIGHUP และ SIGTERM ให้ session CLI แล้ว assert owned masters cleanup in reverse order SIGKILL is documented as non-cleanable and tested only for stale-state reporting
- Status testsแยก master alive, listener bound และ service health โดยไม่อ้างว่า DB/application healthy
- Non-interactive tests close stdin, apply deadlines และ assert no command opens picker, trust confirmation หรือ password prompt
- Cross-platform CI runs behavior suite on macOS and Linux Platform-specific Unix socket length, permissions and process semantics receive explicit cases
- Release tests create real archives, verify checksums and executable mode, then install through mise GitHub backend in isolated mise data directories
- User-server validation is a separate manual acceptance layer It uses authorized environment files without sending secrets into chat and is reported separately from fixture/local results
- The approved test seams are the same CLI, local OpenSSH and mise release seams reviewed in the implementation plan; no further interview is required before implementation

## Out of Scope

- Windows support
- Full-screen TUI, internal web UI และ Raycast plugin
- TypeScript SDK, generated TypeScript client หรือ runtime validator package
- Route ที่มี intermediate hosts มากกว่าหนึ่ง gateway
- ProxyJump-based replacement for the approved gateway LocalForward workflow
- Automatic tunnel reconnect
- Automatic tunnel restore after reboot
- Central daemon or shared team service
- User accounts, team permissions หรือ remote inventory database
- Mandatory migration from plaintext `##PASSWORD` to OS secret storage
- Mandatory conversion of every server to SSH key authentication
- Automatic sharing of one gateway entry across multiple VM entries
- Silent repair of broken metadata after external edits
- Automatic host-key acceptance, replacement หรือ known_hosts deletion
- Automatic service/application health checks beyond transport and listener state
- Generic editing of arbitrary OpenSSH directives through CLI
- Backward compatibility with unusable `seenark/sshx-rust` source, release workflow หรือ legacy annotations
- Modification of the old repository README; the user will mark it deprecated separately

## Further Notes

- Current working directory contains only a mise configuration and is intentionally greenfield
- Rust 1.95.0 is installed through mise; project configuration should pin this exact version instead of floating `latest`
- Sanitized environment audit found 24 reachable config files, 64 exact Host entries, 22 `##PASSWORD` lines, six clear gateway LocalForward candidates and seven localhost VM-like entries
- No current `##PORT`, duplicate alias, wildcard Host, Match or non-Include global directive was found Acceptance for these cases therefore relies on fixtures until real config gains them
- Current config contains many `StrictHostKeyChecking no` directives Approved runtime policy intentionally overrides these without rewriting source files silently
- One existing password-bearing file is not mode 0600 `doctor` must warn but not chmod automatically
- Local experiments proved all current aliases resolve with `ssh -G`, sshpass `-d` works on installed 1.06, wrong-password simulation returns 5, and generated config can replace the transit forward while overriding host-key options
- Local experiments did not prove real user-server authentication, first-use enrollment, host-key rotation or detached lifetime Those results must not be claimed before their respective implementation tests
