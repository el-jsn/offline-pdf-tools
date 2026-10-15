"""Focused local form regression. Run after building the isolated image worker."""
import os, subprocess, tempfile
from pathlib import Path
from reportlab.pdfgen import canvas
from pypdf import PdfReader, PdfWriter
from pypdf.generic import NameObject, TextStringObject, BooleanObject
import pypdfium2 as pdfium
from PIL import ImageChops, ImageStat

ROOT = Path(__file__).resolve().parents[1]
WORK = Path(tempfile.mkdtemp(prefix='pdf-form-regression-', dir='/private/tmp'))
c = canvas.Canvas(str(WORK/'blank.pdf'))
f = c.acroForm
c.drawString(40, 800, 'Synthetic form regression')
for name, y, flags in [('name',740,''),('multiline',650,'multiline'),('aligned',550,'')]:
    c.drawString(40,y+45,name)
    f.textfield(name=name,x=40,y=y,width=300,height=40,fieldFlags=flags,value='Old')
f.checkbox(name='check',x=40,y=480,checked=False)
f.radio(name='radio',value='First',x=100,y=480,selected=True)
f.radio(name='radio',value='Second',x=140,y=480,selected=False)
f.choice(name='choice',x=40,y=400,width=300,height=30,options=['Old','New choice'],value='Old')
c.showPage()
c.drawString(40,800,'Synthetic second page')
f.textfield(name='second-page',x=40,y=700,width=300,height=35,value='Old')
c.save()
r=PdfReader(WORK/'blank.pdf');w=PdfWriter();w.clone_document_from_reader(r)
answers={'name':'Elodie Test 123','multiline':'Line one\nLine two 456','aligned':'Centered 789','choice':'New choice','second-page':'Élodie François'}
for page in w.pages:
    for ref in page.get('/Annots',[]):
        a=ref.get_object();field=a.get('/Parent',a);field=field.get_object()
        name=field.get('/T')
        if name in answers:field[NameObject('/V')]=TextStringObject(answers[name])
        elif name=='check':field[NameObject('/V')]=NameObject('/Yes')
        elif name=='radio':field[NameObject('/V')]=NameObject('/Second')
        if name=='aligned':a[NameObject('/Q')]=__import__('pypdf').generic.NumberObject(1)
w.root_object['/AcroForm'][NameObject('/NeedAppearances')]=BooleanObject(True)
w.write(WORK/'dummy-filled.pdf')
# Build expected rendering with button appearance states matching saved logical values.
for page in w.pages:
    for ref in page.get('/Annots',[]):
        a=ref.get_object();field=a.get('/Parent',a).get_object()
        if field.get('/FT')=='/Btn':
            normal=a.get('/AP',{}).get('/N',{});value=field.get('/V','/Off')
            a[NameObject('/AS')]=NameObject(value if value in normal else '/Off')
w.write(WORK/'expected.pdf')
env=dict(os.environ,PDF_TOOLS_FORM_CHECK=str(WORK))
subprocess.run(['cargo','test','--manifest-path',str(ROOT/'src-tauri/Cargo.toml'),'--bin','offline-pdf-tools','forms::tests::local_filled_form_preparation','--','--ignored'],env=env,check=True)
r=PdfReader(WORK/'verified-flat.pdf')
assert not r.get_fields()
text='\n'.join(p.extract_text() for p in r.pages)
for value in answers.values():
    assert all(line in text for line in value.splitlines()), 'A synthetic answer is missing from flattened text'
expected=pdfium.PdfDocument(str(WORK/'expected.pdf'));expected.init_forms()
actual=pdfium.PdfDocument(str(WORK/'verified-flat.pdf'))
for i in range(len(actual)):
    a=expected[i].render(scale=1.5,may_draw_forms=True).to_pil().convert('RGB')
    b=actual[i].render(scale=1.5).to_pil().convert('RGB')
    difference=ImageChops.difference(a,b)
    mean=sum(ImageStat.Stat(difference).mean)/3
    print('Page',i+1,'mean channel difference:',round(mean,5))
    assert mean<0.15, 'Form rendering differs materially after flattening'
    b.save(WORK/f'page-{i+1}.png')
print('Focused form check passed. Synthetic artifacts:',WORK)
