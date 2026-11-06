package main

import (
	"embed"
	"encoding/json"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
)

//go:generate python3 bundle-skills.py
//go:embed factory-skills.json
var factorySkills embed.FS

// Keep materialized skills beside the shared Git directory so links remain
// readable in the retained Worktree after this opening-half prototype ends.
func linkSkills(common, worktree string) error {
	data, err := factorySkills.ReadFile("factory-skills.json")
	if err != nil {
		return err
	}
	var files map[string]string
	if err := json.Unmarshal(data, &files); err != nil {
		return err
	}
	root, err := os.MkdirTemp(common, "thirdshift-go-skills-")
	if err != nil {
		return err
	}
	for name, content := range files {
		if !fs.ValidPath(name) {
			return fmt.Errorf("invalid embedded skill path %q", name)
		}
		path := filepath.Join(root, filepath.FromSlash(name))
		if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
			return err
		}
		if err := os.WriteFile(path, []byte(content), 0644); err != nil {
			return err
		}
	}
	dir := worktree
	for _, part := range []string{".claude", "skills"} {
		dir = filepath.Join(dir, part)
		if err := os.Mkdir(dir, 0755); err != nil && !os.IsExist(err) {
			return err
		}
		info, err := os.Lstat(dir)
		if err != nil {
			return err
		}
		if !info.IsDir() {
			return fmt.Errorf("skill directory must be a real directory: %s", dir)
		}
	}
	entries, err := os.ReadDir(root)
	if err != nil {
		return err
	}
	for _, skill := range entries {
		if skill.IsDir() && strings.HasPrefix(skill.Name(), "thirdshift-") {
			if err := os.Symlink(filepath.Join(root, skill.Name()), filepath.Join(dir, skill.Name())); err != nil {
				return err
			}
		}
	}
	return nil
}

func excludeFactoryFiles(common string) error {
	path := filepath.Join(common, "info", "exclude")
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return err
	}
	data, err := os.ReadFile(path)
	if err != nil && !os.IsNotExist(err) {
		return err
	}
	file, err := os.OpenFile(path, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0644)
	if err != nil {
		return err
	}
	defer file.Close()
	for _, pattern := range []string{"/.claude/skills/thirdshift-*", "/.thirdshift-issue.md"} {
		if !strings.Contains("\n"+string(data)+"\n", "\n"+pattern+"\n") {
			if _, err := fmt.Fprintln(file, "\n"+pattern); err != nil {
				return err
			}
		}
	}
	return nil
}
