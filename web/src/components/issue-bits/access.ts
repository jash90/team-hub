export function canEdit(project: { role: string | null }): boolean {
  return project.role === 'admin' || project.role === 'member';
}
