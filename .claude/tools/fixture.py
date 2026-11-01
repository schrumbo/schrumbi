import json,os,sys,shutil
R=sys.argv[1]; shutil.rmtree(R,ignore_errors=True); os.makedirs(R)
def rule(i,title,short,wd,folder,pat='*.pdf',opt=False,up=True,time='23:59',interval=1):
    return dict(id=i,title=title,short=short,weekday=wd,time=time,weekOffset=0,optional=opt,upload=up,folder=folder,filePattern=pat,interval=interval)
mods=[
 dict(id='ca1',code='CA1',name='Complex Analysis 1',semester='s1',folder='WS26/Complex Analysis 1',color='blue',naming='{code}_{yy}_{title}_{author}_{date}',passed=False,ects=9,
      rules=[rule('r1','Übungsblatt','UB',5,'Übungen/W{w}'),rule('r2','Quiz','QZ',3,'Quiz',opt=True,up=False)],
      sessions=[dict(id='s1',title='Vorlesung',weekday=2,start='10:15',end='11:45',location='HS 1'),dict(id='s2',title='Übung',weekday=4,start='14:15',end='15:45',location='SR 3')],
      links=[dict(id='l1',title='Moodle',address='https://moodle.example.org/ca1',upload=True),dict(id='l2',title='Skript',address='https://example.org/skript',upload=False),dict(id='l3',title='Forum',address='https://example.org/forum',upload=False)],
      components=[dict(id='c1',title='Klausur',weight=70),dict(id='c2',title='Übungen',weight=30,grade=1.7)],targetGrade=2.0),
 dict(id='prg',code='PRG',name='Programmieren',semester='s1',folder='WS26/Programmieren',color='green',naming='{code}_{title}',passed=False,rules=[],sessions=[],links=[],components=[]),
 dict(id='long',code='EXT',name='Experimentelle Festkörperphysik und angewandte Materialwissenschaften für Fortgeschrittene',semester='s1',folder='WS26/Festkoerper',color='orange',naming='{code}_{title}',passed=False,
      rules=[rule('r3','Protokoll','PR',1,'Protokolle')],sessions=[dict(id='s3',title='Praktikum',weekday=1,start='08:00',end='12:00',location='Labor')],links=[],components=[]),
]
json.dump(dict(name='Modultest',author='Test',version=1,semesters=[dict(id='s1',name='WS 2026/27',start='2026-10-05',end='2027-02-07',breaks=[],holidays=[])],modules=mods,inbox='Inbox'),open(f'{R}/schrumbi.json','w'),ensure_ascii=False,indent=1)
for m in mods: os.makedirs(f"{R}/{m['folder']}",exist_ok=True)
ca=f"{R}/WS26/Complex Analysis 1"
for sub in ['Übungen/W01','Übungen/W02','Vorlesung','Quiz']: os.makedirs(f'{ca}/{sub}',exist_ok=True)
open(f'{ca}/Übungen/W01/CA1_26_Blatt1_Test_261009.pdf','wb').write(b'%PDF-1.4\n%%EOF\n')
for i in range(1,8):
    open(f'{ca}/Vorlesung/Folien {i:02}.pdf','wb').write(b'%PDF-1.4\n%%EOF\n')
    open(f'{ca}/Vorlesung/Mitschrift {i:02}.md','w').write(f'# Mitschrift {i}\n\nHolomorph? :: komplex differenzierbar {i}\n\n')
fs=f"{R}/WS26/Festkoerper"
for i in range(int(sys.argv[2]) if len(sys.argv)>2 else 60): open(f'{fs}/Messung {i:03}.md','w').write(f'# Messung {i}\n')
