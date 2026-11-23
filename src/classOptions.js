export const classSubjects = ['语文', '数学', '英语', '科学', '道德与法治', '历史', '地理', '生物', '物理', '化学', '政治']
const primarySubjects = ['语文', '数学', '英语', '科学']
const juniorSubjects = ['语文', '数学', '英语', '道德与法治', '历史', '地理', '生物']
const seniorSubjects = ['语文', '数学', '英语', '物理', '化学', '生物', '政治', '历史', '地理']
export const classGrades = [
  ...['一', '二', '三', '四', '五', '六'].map(year => ({ name: `小学${year}年级`, stage: '小学', subjects: primarySubjects })),
  { name: '初中一年级', stage: '初中', subjects: juniorSubjects },
  { name: '初中二年级', stage: '初中', subjects: [...juniorSubjects, '物理'] },
  { name: '初中三年级', stage: '初中', subjects: [...juniorSubjects, '物理', '化学'] },
  ...['一', '二', '三'].map(year => ({ name: `高中${year}年级`, stage: '高中', subjects: seniorSubjects })),
]

export const terms = ['上学期', '下学期']
export const termLabel = term => term === '1' ? terms[0] : term === '2' ? terms[1] : term

export function splitSubjects(value = '') {
  return [...new Set(value.split(/[,，]/).map(subject => subject.trim()).filter(Boolean))]
}

export function subjectsForClass(record) {
  const selected = splitSubjects(record?.subjects)
  if (selected.length) return selected
  const fullGrade = (record?.grade ?? '').startsWith(record?.school_stage ?? '') ?
    record?.grade : `${record?.school_stage}${record?.grade}`
  return classGrades.find(grade => grade.name === fullGrade)?.subjects ?? primarySubjects
}

export function nextSchoolTerm(record) {
  const sourceTerm = termLabel(record.term)
  const year = /^(\d{4})\s*[-－]\s*(\d{4})$/.exec(record.school_year)
  return {
    term: sourceTerm === terms[0] ? terms[1] : terms[0],
    school_year: sourceTerm === terms[1] && year ? `${Number(year[1]) + 1}-${Number(year[2]) + 1}` : record.school_year ?? '',
  }
}
